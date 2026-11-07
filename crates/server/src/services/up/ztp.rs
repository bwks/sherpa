//! Phase 8: management addresses, the lab CA and node certificates, and each node's
//! configuration and domain.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::Path;

use anyhow::{Context, Result};
use askama::Template;
use tracing::instrument;

use super::context::LabBuild;
use super::nodes::get_node_image;
use crate::services::guest_account::GuestAccount;
use crate::services::node_ops::{self, DomainParts, LabZtp};
use crate::services::node_resources;
use crate::services::talos;
use crate::services::talos::LabCluster;
use crate::tls::lab_ca::{self, LabCa, NodeCertPaths, NodeIdentity};

use shared::data;
use shared::data::{LabSubnets, RecordId, StatusKind};
use shared::konst::{
    CONTAINER_DNSMASQ_CAPABILITIES, CONTAINER_DNSMASQ_NAME, CONTAINER_DNSMASQ_REPO,
    DNSMASQ_CONFIG_FILE, DNSMASQ_DIR, DNSMASQ_LEASES_FILE, KVM_OUI, LAB_CERTS_DIR,
    MANAGEMENT_NODE_HOST_OFFSETS, MTU_JUMBO_NET, NODE_CONFIGS_DIR, SHERPA_MANAGEMENT_NETWORK_NAME,
    SSH_PORT, TAP_PREFIX, TFTP_DIR, ZTP_DIR,
};
use shared::util;
use topology::find_talos_cluster;

/// What configuring the nodes needs besides the build.
pub(super) struct ConfigInputs<'a> {
    pub(super) node_images: &'a [data::NodeConfig],
    pub(super) node_setup_data: &'a [data::NodeSetupData],
    pub(super) lab_node_data: &'a [data::LabNodeData],
    pub(super) mgmt_net: &'a data::SherpaNetwork,
    pub(super) dns: &'a data::Dns,
    pub(super) subnets: &'a LabSubnets,
    pub(super) sherpa_user: &'a data::User,
    pub(super) personal_account: Option<&'a GuestAccount>,
    pub(super) talos_clusters: &'a [topology::TalosCluster],
}

/// The node configurations and domains, for the phases that create the nodes.
pub(super) struct NodeConfigs {
    pub(super) lab_ca: LabCa,
    pub(super) ztp_dir: String,
    pub(super) tftp_dir: String,
    pub(super) ztp_records: Vec<data::ZtpRecord>,
    /// The volumes each container is created with.
    pub(super) container_volumes: HashMap<String, Vec<String>>,
    pub(super) clone_disks: Vec<data::CloneDisk>,
    pub(super) domains: Vec<template::DomainTemplate>,
    pub(super) unikernel_domains: Vec<template::UnikernelDomainTemplate>,
    pub(super) lab_clusters: Vec<LabCluster>,
}

/// The lab's node configuration in progress: the CA that signs node certificates, and the
/// records and domains made so far.
struct Configuring<'a, 'b> {
    build: &'a LabBuild<'b>,
    inputs: &'a ConfigInputs<'a>,
    lab_ca: LabCa,
    certs_dir: String,
    tftp_dir: String,
    ztp_records: Vec<data::ZtpRecord>,
    clone_disks: Vec<data::CloneDisk>,
}

/// Give every node its management addresses and TLS certificate, and generate its
/// configuration: container volumes, Kubernetes cluster secrets and machine configs, VM boot
/// configs, disks and domains, and unikernel domains (phase 8).
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn configure_nodes(
    build: &LabBuild<'_>,
    inputs: &ConfigInputs<'_>,
    container_nodes: &mut [topology::NodeExpanded],
    vm_nodes: &mut [topology::NodeExpanded],
    unikernel_nodes: &mut [topology::NodeExpanded],
) -> Result<NodeConfigs> {
    let lab_id = build.lab_id;
    let ztp_dir = format!("{}/{ZTP_DIR}", build.lab_dir);
    let tftp_dir = format!("{ztp_dir}/{TFTP_DIR}");
    util::create_dir(&ztp_dir)?;
    util::create_dir(&tftp_dir)?;
    let certs_dir = format!("{}/{LAB_CERTS_DIR}", build.lab_dir);
    util::create_dir(&certs_dir)?;
    let lab_ca = lab_ca::create_lab_ca(Path::new(&certs_dir), lab_id)
        .context("Failed to generate lab CA certificate")?;
    let _ = build.progress.send_status(
        format!("Lab CA certificate generated for lab: {lab_id}"),
        StatusKind::Done,
    );

    let mut configuring = Configuring {
        build,
        inputs,
        lab_ca,
        certs_dir,
        tftp_dir,
        ztp_records: Vec::new(),
        clone_disks: Vec::new(),
    };
    let mut container_volumes = HashMap::new();
    for node in container_nodes.iter_mut() {
        let volumes = configuring.configure_container(node).await?;
        container_volumes.insert(node.name.clone(), volumes);
    }

    tracing::info!(
        vm_count = vm_nodes.len(),
        "Generating VM configurations and domains"
    );
    let lab_clusters = configuring.plan_clusters(vm_nodes)?;
    let mut domains = Vec::with_capacity(vm_nodes.len());
    for node in vm_nodes.iter_mut() {
        domains.push(configuring.configure_vm(node, &lab_clusters).await?);
    }
    let mut unikernel_domains = Vec::with_capacity(unikernel_nodes.len());
    for node in unikernel_nodes.iter_mut() {
        unikernel_domains.push(configuring.configure_unikernel(node).await?);
    }

    let Configuring {
        lab_ca,
        tftp_dir,
        ztp_records,
        clone_disks,
        ..
    } = configuring;
    Ok(NodeConfigs {
        lab_ca,
        ztp_dir,
        tftp_dir,
        ztp_records,
        container_volumes,
        clone_disks,
        domains,
        unikernel_domains,
        lab_clusters,
    })
}

impl Configuring<'_, '_> {
    /// The lab's part of generating every node's configuration.
    fn lab_ztp(&self) -> LabZtp<'_> {
        LabZtp {
            lab_id: self.build.lab_id,
            lab_dir: self.build.lab_dir,
            images_dir: &self.build.config.images_dir,
            sherpa_user: self.inputs.sherpa_user,
            dns: self.inputs.dns,
            mgmt_net: self.inputs.mgmt_net,
            progress: self.build.progress,
            owner_account: self.inputs.personal_account,
        }
    }

    /// Give `node` its management addresses, from its index, and record them. Returns its IPv4
    /// address.
    async fn assign_addresses(&self, node: &mut topology::NodeExpanded) -> Result<Ipv4Addr> {
        let mgmt_net = self.inputs.mgmt_net;
        let node_idx = node_ops::get_node_data(&node.name, self.inputs.node_setup_data)?.index;
        let host_offset = util::node_management_host_offset(node_idx)?;
        let ipv4 = util::get_ipv4_addr(&mgmt_net.v4.prefix, host_offset)?;
        node.ipv4_address = Some(ipv4);
        if let Some(v6) = &mgmt_net.v6 {
            node.ipv6_address = Some(util::get_ipv6_addr(&v6.prefix, host_offset)?);
        }
        if let Some(record_id) = self.record_id(&node.name)? {
            let db = self.build.db;
            db::update_node_mgmt_ipv4(db, record_id.clone(), &ipv4.to_string()).await?;
            if let Some(ipv6) = node.ipv6_address {
                db::update_node_mgmt_ipv6(db, record_id, &ipv6.to_string()).await?;
            }
        }
        Ok(ipv4)
    }

    /// Issue `node`'s TLS certificate from the lab CA.
    fn issue_certificate(
        &self,
        node: &topology::NodeExpanded,
        ipv4: Ipv4Addr,
    ) -> Result<NodeCertPaths> {
        lab_ca::issue_node_certificate(
            &self.lab_ca,
            Path::new(&self.certs_dir),
            NodeIdentity {
                name: &node.name,
                lab_id: self.build.lab_id,
                ipv4,
                ipv6: node.ipv6_address,
            },
        )
    }

    /// The database record of node `name`, if it has one.
    fn record_id(&self, name: &str) -> Result<Option<RecordId>> {
        self.inputs
            .lab_node_data
            .iter()
            .find(|node| node.name == name)
            .map(|node| db::get_node_id(&node.record))
            .transpose()
    }

    async fn record_mac(&self, name: &str, mac_address: &str) -> Result<()> {
        if let Some(record_id) = self.record_id(name)? {
            db::update_node_mgmt_mac(self.build.db, record_id, mac_address).await?;
        }
        Ok(())
    }

    fn image(&self, node: &topology::NodeExpanded) -> Result<data::NodeConfig> {
        get_node_image(node.model, node.version.as_deref(), self.inputs.node_images)
    }

    /// Configure a container node. Returns the volumes it is created with: its own and the lab
    /// certificate mounts.
    async fn configure_container(
        &mut self,
        node: &mut topology::NodeExpanded,
    ) -> Result<Vec<String>> {
        decode_ztp_config(node)?;
        let node_image = self.image(node)?;
        let ipv4 = self.assign_addresses(node).await?;
        let cert_paths = self.issue_certificate(node, ipv4)?;
        let ztp_result = node_ops::generate_container_ztp(
            node,
            &node_image,
            &self.lab_ztp(),
            ipv4,
            Some(&cert_paths),
        )?;
        self.ztp_records.push(ztp_result.ztp_record);
        Ok(ztp_result.volumes)
    }

    /// Plan the lab's Kubernetes clusters and generate their secrets, before any node config.
    fn plan_clusters(&self, vm_nodes: &[topology::NodeExpanded]) -> Result<Vec<LabCluster>> {
        let talos_clusters = self.inputs.talos_clusters;
        if talos_clusters.is_empty() {
            return Ok(Vec::new());
        }
        let mut members = Vec::new();
        for cluster in talos_clusters {
            for (name, role) in cluster.members() {
                let node = vm_nodes
                    .iter()
                    .find(|node| node.name == name)
                    .with_context(|| format!("Talos cluster member '{name}' is not a VM node"))?;
                let image = self.image(node)?;
                talos::secrets::ensure_supported_version(name, &image.version)?;
                members.push(talos::plan::MemberInput {
                    name,
                    cluster: &cluster.name,
                    role,
                    ipv4: self.planned_ipv4(name)?,
                });
            }
        }
        // The Keycloak and Vault nodes clusters use, with their addresses.
        let mut services: Vec<talos::plan::ClusterMember> = Vec::new();
        for cluster in talos_clusters {
            for name in [&cluster.oidc, &cluster.vault].into_iter().flatten() {
                if services.iter().any(|service| &service.name == name) {
                    continue;
                }
                services.push(talos::plan::ClusterMember {
                    name: name.clone(),
                    ipv4: self.planned_ipv4(name)?,
                });
            }
        }
        let clusters = talos::prepare_clusters(
            self.build.lab_id,
            talos_clusters,
            &members,
            &services,
            &self.inputs.mgmt_net.v4,
        )?;
        let _ = self.build.progress.send_status(
            format!(
                "Generated secrets for {} Kubernetes clusters",
                clusters.len()
            ),
            StatusKind::Done,
        );
        Ok(clusters)
    }

    /// The management IPv4 address node `name` will get.
    fn planned_ipv4(&self, name: &str) -> Result<Ipv4Addr> {
        let node_idx = node_ops::get_node_data(name, self.inputs.node_setup_data)?.index;
        util::get_ipv4_addr(
            &self.inputs.mgmt_net.v4.prefix,
            util::node_management_host_offset(node_idx)?,
        )
    }

    /// Configure a VM node: its boot config, disks to clone, and domain.
    async fn configure_vm(
        &mut self,
        node: &mut topology::NodeExpanded,
        lab_clusters: &[LabCluster],
    ) -> Result<template::DomainTemplate> {
        decode_ztp_config(node)?;
        // Cluster nodes get a generated machine config (validation forbids their own).
        if let Some((cluster, _)) = find_talos_cluster(self.inputs.talos_clusters, &node.name) {
            let cluster = talos::find_cluster(lab_clusters, &cluster.name)?;
            node.ztp_config = Some(talos::node_machine_config(
                cluster,
                &node.name,
                &self.inputs.mgmt_net.v4,
                &self.lab_ca.pair.cert,
            )?);
        }
        let node_data = node_ops::get_node_data(&node.name, self.inputs.node_setup_data)?;
        let node_image = self.image(node)?;
        let ipv4 = self.assign_addresses(node).await?;
        let cert_paths = self.issue_certificate(node, ipv4)?;
        let ztp_result = node_ops::generate_vm_ztp(
            node,
            &node_image,
            &self.lab_ztp(),
            ipv4,
            None,
            Some(&cert_paths),
        )?;
        self.record_mac(&node.name, &ztp_result.mac_address).await?;
        self.ztp_records.push(ztp_result.ztp_record);
        self.clone_disks.extend(ztp_result.clone_disks);

        let interfaces = domain_interfaces(
            node,
            &node_data,
            &node_image,
            &ztp_result.mac_address,
            self.build.lab_id,
        )?;
        let record = self
            .inputs
            .lab_node_data
            .iter()
            .find(|n| n.name == node.name)
            .map(|n| &n.record)
            .with_context(|| format!("Node '{}' has no database record", node.name))?;
        let loopback_host = u8::try_from(node_data.index).with_context(|| {
            format!(
                "Node '{}' has an index past the loopback network",
                node.name
            )
        })?;
        node_ops::build_domain_template(
            node,
            &node_image,
            self.build.lab_id,
            &self.build.config.qemu_bin,
            DomainParts {
                disks: ztp_result.disks,
                interfaces,
                loopback_ipv4: util::get_ip(&self.inputs.subnets.loopback_subnet, loopback_host)
                    .to_string(),
                management_network: node_data.management_network.clone(),
                isolated_network: node_data
                    .isolated_network
                    .as_ref()
                    .map(|net| net.network_name.clone())
                    .unwrap_or_default(),
                reserved_network: node_data
                    .reserved_network
                    .as_ref()
                    .map(|net| net.network_name.clone())
                    .unwrap_or_default(),
            },
            ztp_result.qemu_commands,
            &node_resources::resources_of(record),
        )
    }

    /// Configure a unikernel node: its disk or kernel, a DHCP host entry, and its domain.
    async fn configure_unikernel(
        &mut self,
        node: &mut topology::NodeExpanded,
    ) -> Result<template::UnikernelDomainTemplate> {
        let node_data = node_ops::get_node_data(&node.name, self.inputs.node_setup_data)?;
        let node_image = self.image(node)?;
        let ipv4 = self.assign_addresses(node).await?;
        self.issue_certificate(node, ipv4)?;
        let setup_result = node_ops::generate_unikernel_setup(
            node,
            &node_image,
            self.build.lab_id,
            &self.build.config.images_dir,
        )?;
        self.record_mac(&node.name, &setup_result.mac_address)
            .await?;
        // Unikernels have no boot file, only the DHCP host entry (MAC to IP).
        self.ztp_records.push(data::ZtpRecord {
            node_name: node.name.clone(),
            config_file: String::new(),
            ipv4_address: ipv4,
            ipv6_address: node.ipv6_address,
            mac_address: setup_result.mac_address.clone(),
            ztp_method: data::ZtpMethod::None,
            ssh_port: Some(SSH_PORT),
        });
        self.clone_disks.extend(setup_result.clone_disks);

        let interfaces = domain_interfaces(
            node,
            &node_data,
            &node_image,
            &setup_result.mac_address,
            self.build.lab_id,
        )?;
        let loopback_host = u8::try_from(node_data.index).with_context(|| {
            format!(
                "Node '{}' has an index past the loopback network",
                node.name
            )
        })?;
        let domain = node_ops::build_unikernel_domain_template(
            node,
            &node_image,
            self.build.lab_id,
            &self.build.config.qemu_bin,
            DomainParts {
                disks: setup_result.disks,
                interfaces,
                loopback_ipv4: util::get_ip(&self.inputs.subnets.loopback_subnet, loopback_host)
                    .to_string(),
                management_network: node_data.management_network.clone(),
                isolated_network: node_data
                    .isolated_network
                    .as_ref()
                    .map(|net| net.network_name.clone())
                    .unwrap_or_default(),
                reserved_network: node_data
                    .reserved_network
                    .as_ref()
                    .map(|net| net.network_name.clone())
                    .unwrap_or_default(),
            },
            setup_result.kernel_path,
        );
        let _ = self.build.progress.send_status(
            format!("Unikernel node {} configured", node.name),
            StatusKind::Done,
        );
        Ok(domain)
    }
}

/// Replace the client's base64 `ztp_config` on `node` with the config itself.
fn decode_ztp_config(node: &mut topology::NodeExpanded) -> Result<()> {
    if let Some(encoded) = &node.ztp_config {
        let decoded = util::base64_decode(encoded)
            .with_context(|| format!("Failed to decode ztp_config for node '{}'", node.name))?;
        node.ztp_config = Some(decoded);
    }
    Ok(())
}

/// The interfaces of a VM or unikernel domain, in index order: the management interface with
/// `mac_address`, then reserved, bridge, P2p and disabled ones.
fn domain_interfaces(
    node: &topology::NodeExpanded,
    node_data: &data::NodeSetupData,
    node_image: &data::NodeConfig,
    mac_address: &str,
    lab_id: &str,
) -> Result<Vec<data::Interface>> {
    let mut interfaces = Vec::with_capacity(node_data.interfaces.len());
    for interface in &node_data.interfaces {
        let (name, mtu, mac, connection_type) = match &interface.data {
            data::NodeInterface::Management => (
                util::dasher(&node_image.management_interface.to_string()),
                node_image.interface_mtu,
                mac_address.to_owned(),
                data::ConnectionTypes::Management,
            ),
            data::NodeInterface::Reserved => (
                format!("int{}", interface.index),
                node_image.interface_mtu,
                util::random_mac(KVM_OUI),
                data::ConnectionTypes::Reserved,
            ),
            data::NodeInterface::Bridge(bridge) => (
                bridge.name.clone(),
                node_image.interface_mtu,
                util::random_mac(KVM_OUI),
                data::ConnectionTypes::PrivateBridge,
            ),
            data::NodeInterface::Peer(peer) => {
                let side = match peer.this_side {
                    data::PeerSide::A => "a",
                    data::PeerSide::B => "b",
                };
                (
                    format!("{TAP_PREFIX}{side}{}-{lab_id}", peer.link_index),
                    MTU_JUMBO_NET,
                    util::random_mac(KVM_OUI),
                    data::ConnectionTypes::P2p,
                )
            }
            data::NodeInterface::Disabled => (
                util::dasher(&util::interface_from_idx(&node.model, interface.index)?),
                node_image.interface_mtu,
                util::random_mac(KVM_OUI),
                data::ConnectionTypes::Disabled,
            ),
        };
        interfaces.push(data::Interface {
            name,
            num: interface.index,
            mtu,
            mac_address: mac,
            connection_type,
        });
    }
    Ok(interfaces)
}

/// Write the dnsmasq config, DHCP host entries included, and start the lab's boot container
/// (phase 9): DHCP, DNS and TFTP for the management network, and the node configs over HTTP.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn start_boot_container(
    build: &LabBuild<'_>,
    ztp_dir: &str,
    tftp_dir: &str,
    mgmt_net: &data::SherpaNetwork,
    ztp_records: &[data::ZtpRecord],
) -> Result<()> {
    let LabBuild {
        lab_id,
        docker_conn,
        ..
    } = *build;

    tracing::info!(lab_id = %lab_id, "Creating ZTP boot infrastructure");

    // `ztp_dir` and `tftp_dir` were made in phase 8.
    let ztp_configs_dir = format!("{ztp_dir}/{NODE_CONFIGS_DIR}");
    let dnsmasq_dir = format!("{ztp_dir}/{DNSMASQ_DIR}");
    util::create_dir(&ztp_configs_dir)?;
    util::create_dir(&dnsmasq_dir)?;

    tracing::debug!(
        lab_id = %lab_id,
        ztp_dir = %ztp_dir,
        tftp_dir = %tftp_dir,
        dnsmasq_dir = %dnsmasq_dir,
        "Created ZTP directories"
    );

    write_dnsmasq_config(&dnsmasq_dir, mgmt_net, ztp_records)?;

    // Create boot container (tftp_dir already created in Phase 8)
    let configs_dir = format!("{ztp_dir}/{NODE_CONFIGS_DIR}");

    let dnsmasq_env_dns1 = format!("DNS1={}", mgmt_net.v4.first);
    let dnsmasq_env_dns2 = "DNS2=".to_string();
    let boot_server_ipv4 = mgmt_net.v4.boot_server.to_string();

    let webdir_config_volume = format!("{configs_dir}:/opt/{ZTP_DIR}/{NODE_CONFIGS_DIR}");
    let dnsmasq_env_vars = vec![dnsmasq_env_dns1, dnsmasq_env_dns2];
    let dnsmasq_config_volume =
        format!("{dnsmasq_dir}/{DNSMASQ_CONFIG_FILE}:/etc/{DNSMASQ_CONFIG_FILE}");
    let dnsmasq_tftp_volume = format!("{tftp_dir}:/opt/{ZTP_DIR}/{TFTP_DIR}");
    let dnsmasq_volumes = vec![
        dnsmasq_config_volume,
        dnsmasq_tftp_volume,
        webdir_config_volume,
    ];
    let dnsmasq_capabilities: Vec<String> = CONTAINER_DNSMASQ_CAPABILITIES
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    let management_network_attachment = data::ContainerNetworkAttachment {
        name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
        ipv4_address: Some(boot_server_ipv4.clone()),
        ipv6_address: None,
        linux_interface_name: None,
        admin_down: false,
    };

    tracing::info!(
        lab_id = %lab_id,
        container = %format!("{CONTAINER_DNSMASQ_NAME}-{lab_id}"),
        boot_server_ip = %boot_server_ipv4,
        "Starting dnsmasq boot container"
    );

    let is_running = container::run_container(
        docker_conn,
        container::ContainerSpec {
            name: format!("{CONTAINER_DNSMASQ_NAME}-{lab_id}"),
            image: CONTAINER_DNSMASQ_REPO.to_string(),
            env_vars: dnsmasq_env_vars,
            volumes: dnsmasq_volumes,
            capabilities: dnsmasq_capabilities,
            management_network: management_network_attachment,
            additional_networks: vec![],
            commands: vec![],
            privileged: false,
            shm_size: None,
            user: None,
        },
    )
    .await?;

    if !is_running {
        anyhow::bail!(
            "dnsmasq boot container {CONTAINER_DNSMASQ_NAME}-{lab_id} is not in running state after start"
        );
    }

    Ok(())
}

/// Write the dnsmasq config, with a DHCP host entry for each node, and its empty leases file.
#[instrument(skip_all, level = "debug")]
fn write_dnsmasq_config(
    dnsmasq_dir: &str,
    mgmt_net: &data::SherpaNetwork,
    ztp_records: &[data::ZtpRecord],
) -> Result<()> {
    let dnsmaq_template = template::DnsmasqTemplate {
        tftp_server_ipv4: mgmt_net.v4.boot_server.to_string(),
        gateway_ipv4: mgmt_net.v4.first.to_string(),
        dhcp_start: util::get_ipv4_addr(
            &mgmt_net.v4.prefix,
            *MANAGEMENT_NODE_HOST_OFFSETS.start(),
        )?
        .to_string(),
        dhcp_end: util::get_ipv4_addr(&mgmt_net.v4.prefix, *MANAGEMENT_NODE_HOST_OFFSETS.end())?
            .to_string(),
        gateway_ipv6: mgmt_net.v6.as_ref().map(|v6| v6.first.to_string()),
        dhcp6_start: mgmt_net
            .v6
            .as_ref()
            .map(|v6| {
                util::get_ipv6_addr(&v6.prefix, *MANAGEMENT_NODE_HOST_OFFSETS.start())
                    .map(|a| a.to_string())
            })
            .transpose()?,
        dhcp6_end: mgmt_net
            .v6
            .as_ref()
            .map(|v6| {
                util::get_ipv6_addr(&v6.prefix, *MANAGEMENT_NODE_HOST_OFFSETS.end())
                    .map(|a| a.to_string())
            })
            .transpose()?,
        dns_ipv6: mgmt_net.v6.as_ref().map(|v6| v6.boot_server.to_string()),
        ztp_records: ztp_records.to_vec(),
    };
    let dnsmasq_rendered_template = dnsmaq_template.render()?;
    util::create_file(
        &format!("{dnsmasq_dir}/{DNSMASQ_CONFIG_FILE}"),
        dnsmasq_rendered_template,
    )?;
    util::create_file(
        &format!("{dnsmasq_dir}/{DNSMASQ_LEASES_FILE}"),
        String::new(),
    )?;
    Ok(())
}
