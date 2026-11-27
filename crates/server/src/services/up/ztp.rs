//! Phase 8: management addresses, the lab CA and node certificates, and each node's
//! configuration and domain.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::instrument;

use super::context::LabBuild;
use super::nodes::get_node_image;
use crate::konst::{
    DNSMASQ_CONFIG_FILE, LAB_CERTS_DIR, NODE_CONFIG_DIR_MODE, TAP_PREFIX, TFTP_DIR_MODE,
};
use crate::services::boot_server;
use crate::services::console;
use crate::services::guest_account::GuestAccount;
use crate::services::node_ops::{self, ContainerSettings, DomainParts, LabZtp};
use crate::services::node_resources;
use crate::services::talos;
use crate::services::talos::LabCluster;
use crate::tls::lab_ca::{self, LabCa, NodeCertPaths, NodeIdentity};

use shared::data;
use shared::data::{ClusterMember, RecordId, StatusKind};
use shared::konst::{
    DNSMASQ_DIR, KVM_OUI, MTU_JUMBO_NET, NODE_CONFIGS_DIR, SSH_PORT, TFTP_DIR, ZTP_DIR,
};
use shared::util;
use topology::find_talos_cluster;

/// What configuring the nodes needs besides the build.
pub(super) struct ConfigInputs<'a> {
    pub(super) lab_record_id: &'a RecordId,
    pub(super) node_images: &'a [data::NodeConfig],
    pub(super) node_setup_data: &'a [data::NodeSetupData],
    pub(super) lab_node_data: &'a [data::LabNodeData],
    pub(super) mgmt_net: &'a data::SherpaNetwork,
    pub(super) dns: &'a data::Dns,
    pub(super) owner_account: &'a GuestAccount,
    pub(super) talos_clusters: &'a [topology::TalosCluster],
}

/// The node configurations and domains, for the phases that create the nodes.
pub(super) struct NodeConfigs {
    pub(super) lab_ca: LabCa,
    pub(super) ztp_dir: String,
    pub(super) ztp_records: Vec<data::ZtpRecord>,
    pub(super) container_settings: HashMap<String, ContainerSettings>,
    pub(super) clone_disks: Vec<data::CloneDisk>,
    pub(super) domains: Vec<template::DomainTemplate>,
    pub(super) unikernel_domains: Vec<template::UnikernelDomainTemplate>,
    pub(super) lab_clusters: Vec<LabCluster>,
    pub(super) sandbox_records: Vec<SandboxRecord>,
}

/// The record of the sandbox of node `node_name`.
#[derive(Clone, Debug)]
pub(super) struct SandboxRecord {
    pub(super) node_name: String,
    pub(super) record: data::DbVmSandbox,
}

/// The lab's node configuration in progress: the CA that signs node certificates, and the
/// records and domains made so far.
struct Configuring<'a, 'b> {
    build: &'a LabBuild<'b>,
    inputs: &'a ConfigInputs<'a>,
    lab_ca: LabCa,
    certs_dir: String,
    ztp_records: Vec<data::ZtpRecord>,
    clone_disks: Vec<data::CloneDisk>,
    node_count: u16,
    sandbox_ranks: HashMap<String, u16>,
    sandbox_records: Vec<SandboxRecord>,
}

/// Give every node its management addresses, every container and VM its TLS certificate, and
/// generate each node's configuration: container volumes, Kubernetes cluster secrets and
/// machine configs, VM boot configs, disks and domains, and unikernel domains (phase 8).
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
    util::create_dir_with_permissions(Path::new(&ztp_dir), NODE_CONFIG_DIR_MODE)?;
    util::create_dir_with_permissions(Path::new(&tftp_dir), TFTP_DIR_MODE)?;
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
        ztp_records: Vec::new(),
        clone_disks: Vec::new(),
        node_count: u16::try_from(inputs.node_setup_data.len())
            .context("The lab has more nodes than it can number")?,
        sandbox_ranks: sandbox_ranks(container_nodes),
        sandbox_records: Vec::new(),
    };
    let mut container_settings = HashMap::new();
    for node in container_nodes.iter_mut() {
        let settings = configuring.configure_container(node).await?;
        container_settings.insert(node.name.clone(), settings);
    }

    tracing::info!(
        vm_count = vm_nodes.len(),
        "Generating VM configurations and domains"
    );
    let lab_clusters = configuring.plan_clusters(vm_nodes).await?;
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
        ztp_records,
        clone_disks,
        sandbox_records,
        ..
    } = configuring;
    Ok(NodeConfigs {
        lab_ca,
        ztp_dir,
        ztp_records,
        container_settings,
        clone_disks,
        domains,
        unikernel_domains,
        lab_clusters,
        sandbox_records,
    })
}

/// The rank of each of `container_nodes` that runs in a sandbox, from 1 in node index order,
/// which places its sandbox's own address after the nodes' addresses.
fn sandbox_ranks(container_nodes: &[topology::NodeExpanded]) -> HashMap<String, u16> {
    let mut sandboxes: Vec<&topology::NodeExpanded> = container_nodes
        .iter()
        .filter(|node| node.model_kind.runs_in_sandbox())
        .collect();
    sandboxes.sort_by_key(|node| node.index);
    (1..=u16::MAX)
        .zip(sandboxes)
        .map(|(rank, node)| (node.name.clone(), rank))
        .collect()
}

impl Configuring<'_, '_> {
    /// The lab's part of generating every node's configuration.
    fn lab_ztp(&self) -> LabZtp<'_> {
        LabZtp {
            lab_id: self.build.lab_id,
            lab_dir: self.build.lab_dir,
            images_dir: &self.build.config.images_dir,
            dns: self.inputs.dns,
            mgmt_net: self.inputs.mgmt_net,
            progress: self.build.progress,
            owner_account: self.inputs.owner_account,
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

    /// Give the sandbox of node `name`, the `rank`th of the lab, its own management addresses,
    /// after the nodes' addresses, and its NIC's MAC, and record them.
    async fn assign_sandbox(&mut self, name: &str, rank: u16) -> Result<()> {
        let mgmt_net = self.inputs.mgmt_net;
        let offset = util::sandbox_management_host_offset(self.node_count, rank)?;
        let record = data::DbVmSandbox {
            node: self
                .record_id(name)?
                .with_context(|| format!("Node '{name}' has no record"))?,
            ipv4: util::get_ipv4_addr(&mgmt_net.v4.prefix, offset)?,
            ipv6: mgmt_net
                .v6
                .as_ref()
                .map(|v6| util::get_ipv6_addr(&v6.prefix, offset))
                .transpose()?,
            mac: util::random_mac(KVM_OUI),
            ssh_host_key: None,
        };
        db::create_vm_sandbox(self.build.db, &record).await?;
        self.sandbox_records.push(SandboxRecord {
            node_name: name.to_owned(),
            record,
        });
        Ok(())
    }

    async fn record_mac(&self, name: &str, mac_address: &str) -> Result<()> {
        if let Some(record_id) = self.record_id(name)? {
            db::update_node_mgmt_mac(self.build.db, record_id, mac_address).await?;
        }
        Ok(())
    }

    fn image(&self, node: &topology::NodeExpanded) -> Result<data::NodeConfig> {
        get_node_image(
            node.model_kind,
            node.version.as_deref(),
            self.inputs.node_images,
        )
    }

    /// Configure a container node and return the settings it is created with, its mounts with
    /// the lab certificate mounts.
    async fn configure_container(
        &mut self,
        node: &mut topology::NodeExpanded,
    ) -> Result<ContainerSettings> {
        decode_ztp_config(node)?;
        let node_image = self.image(node)?;
        let ipv4 = self.assign_addresses(node).await?;
        if let Some(&rank) = self.sandbox_ranks.get(&node.name) {
            self.assign_sandbox(&node.name, rank).await?;
        }
        let cert_paths = self.issue_certificate(node, ipv4)?;
        let ztp_result = node_ops::generate_container_ztp(
            node,
            &node_image,
            &self.lab_ztp(),
            ipv4,
            Some(&cert_paths),
        )?;
        self.ztp_records.push(ztp_result.ztp_record);
        Ok(ztp_result.settings)
    }

    /// Plan the lab's Kubernetes clusters, store their plans and generate their secrets, before
    /// any node config.
    async fn plan_clusters(&self, vm_nodes: &[topology::NodeExpanded]) -> Result<Vec<LabCluster>> {
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
        let mut services: Vec<ClusterMember> = Vec::new();
        for cluster in talos_clusters {
            for name in [&cluster.oidc, &cluster.vault].into_iter().flatten() {
                if services.iter().any(|service| &service.name == name) {
                    continue;
                }
                services.push(ClusterMember {
                    name: name.clone(),
                    ipv4: self.planned_ipv4(name)?,
                });
            }
        }
        let clusters = talos::prepare_clusters(
            self.build.db,
            self.inputs.lab_record_id,
            self.build.lab_id,
            talos_clusters,
            &members,
            &services,
            &self.inputs.mgmt_net.v4,
        )
        .await?;
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
            node.ztp.config = Some(talos::node_machine_config(
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
        node_ops::build_domain_template(
            node,
            &node_image,
            self.build.lab_id,
            &self.build.config.qemu_bin,
            DomainParts {
                disks: ztp_result.disks,
                interfaces,
                consoles: console::console_sockets(self.build.lab_id, node_data.index)?,
                management_network: node_data.management_network.clone(),
                isolated_network: node_data
                    .isolated_network
                    .as_ref()
                    .map_or_default(|net| net.network_name.clone()),
                reserved_network: node_data
                    .reserved_network
                    .as_ref()
                    .map_or_default(|net| net.network_name.clone()),
            },
            ztp_result.firmware,
            &node_resources::resources_of(record),
        )
    }

    /// Configure a unikernel node: its disk or kernel, a DHCP host entry, and its domain. It gets
    /// no certificate: it has no boot config or mount to carry one.
    async fn configure_unikernel(
        &mut self,
        node: &mut topology::NodeExpanded,
    ) -> Result<template::UnikernelDomainTemplate> {
        let node_data = node_ops::get_node_data(&node.name, self.inputs.node_setup_data)?;
        let node_image = self.image(node)?;
        let ipv4 = self.assign_addresses(node).await?;
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
            ztp: data::Ztp::Disabled,
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
        let domain = node_ops::build_unikernel_domain_template(
            node,
            &node_image,
            self.build.lab_id,
            &self.build.config.qemu_bin,
            DomainParts {
                disks: setup_result.disks,
                interfaces,
                consoles: console::console_sockets(self.build.lab_id, node_data.index)?,
                management_network: node_data.management_network.clone(),
                isolated_network: node_data
                    .isolated_network
                    .as_ref()
                    .map_or_default(|net| net.network_name.clone()),
                reserved_network: node_data
                    .reserved_network
                    .as_ref()
                    .map_or_default(|net| net.network_name.clone()),
            },
            setup_result.kernel_path,
        )?;
        let _ = self.build.progress.send_status(
            format!("Unikernel node {} configured", node.name),
            StatusKind::Done,
        );
        Ok(domain)
    }
}

/// Replace the client's base64 `ztp.config` on `node` with the config itself.
fn decode_ztp_config(node: &mut topology::NodeExpanded) -> Result<()> {
    if let Some(encoded) = &node.ztp.config {
        let decoded = util::base64_decode(encoded)
            .with_context(|| format!("Failed to decode ztp.config for node '{}'", node.name))?;
        node.ztp.config = Some(decoded);
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
    let image_mtu = node_image
        .interface_device()
        .with_context(|| format!("Node image {} is a container's", node_image.model))?
        .mtu;
    let mut interfaces = Vec::with_capacity(node_data.interfaces.len());
    for interface in &node_data.interfaces {
        let (name, mtu, mac, connection_type) = match &interface.data {
            data::NodeInterface::Management => (
                util::dasher(&node_image.interfaces.management.to_string()),
                image_mtu,
                mac_address.to_owned(),
                data::ConnectionTypes::Management,
            ),
            data::NodeInterface::Reserved => (
                format!("int{}", interface.index),
                image_mtu,
                util::random_mac(KVM_OUI),
                data::ConnectionTypes::Reserved,
            ),
            data::NodeInterface::Bridge(bridge) => (
                bridge.name.clone(),
                image_mtu,
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
                util::dasher(&util::interface_from_idx(node.model_kind, interface.index)?),
                image_mtu,
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
/// (phase 9): DHCP, DNS and TFTP for the management network, and the HTTP boot files.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn start_boot_container(
    build: &LabBuild<'_>,
    ztp_dir: &str,
    mgmt_net: &data::SherpaNetwork,
    ztp_records: &[data::ZtpRecord],
    sandbox_records: &[SandboxRecord],
) -> Result<()> {
    let LabBuild {
        lab_id,
        docker_conn,
        ..
    } = *build;

    tracing::info!(lab_id = %lab_id, "Creating ZTP boot infrastructure");

    // `ztp_dir` and its TFTP folder were made in phase 8.
    let ztp_configs_dir = format!("{ztp_dir}/{NODE_CONFIGS_DIR}");
    let dnsmasq_dir = format!("{ztp_dir}/{DNSMASQ_DIR}");
    util::create_dir_with_permissions(Path::new(&ztp_configs_dir), NODE_CONFIG_DIR_MODE)?;
    util::create_dir(&dnsmasq_dir)?;

    tracing::debug!(
        lab_id = %lab_id,
        ztp_dir = %ztp_dir,
        dnsmasq_dir = %dnsmasq_dir,
        "Created ZTP directories"
    );

    let sandbox_hosts: Vec<template::SandboxHostRecord> = sandbox_records
        .iter()
        .map(|sandbox| boot_server::sandbox_host_record(&sandbox.node_name, &sandbox.record))
        .collect();
    write_dnsmasq_config(&dnsmasq_dir, mgmt_net, ztp_records, &sandbox_hosts)?;
    boot_server::run_boot_container(docker_conn, lab_id, ztp_dir, mgmt_net).await
}

/// Write the dnsmasq config, with a DHCP host entry for each node and for the own address of each
/// sandbox, as the only file mounted in the boot container, which keeps dnsmasq's leases.
#[instrument(skip_all, level = "debug")]
fn write_dnsmasq_config(
    dnsmasq_dir: &str,
    mgmt_net: &data::SherpaNetwork,
    ztp_records: &[data::ZtpRecord],
    sandbox_hosts: &[template::SandboxHostRecord],
) -> Result<()> {
    util::create_file(
        &format!("{dnsmasq_dir}/{DNSMASQ_CONFIG_FILE}"),
        boot_server::render_dnsmasq_config(mgmt_net, ztp_records, sandbox_hosts)?,
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use bollard::Docker;
    use libvirt::Qemu;
    use tokio::sync::mpsc;

    use crate::daemon::sandbox_disks::SandboxDiskLocks;
    use crate::konst::{LAB_CA_CERT_FILE, LAB_CA_KEY_FILE};
    use shared::data::{LabSubnets, ModelKind, NodeKind, NodeModel};

    use super::super::networks::management_network;
    use super::*;
    use crate::services::guest_account::tests::test_account;
    use crate::services::lab_definition::tests::lab_with;
    use crate::services::progress::ProgressSender;

    fn subnets() -> LabSubnets {
        LabSubnets {
            management_subnet: "172.31.1.0/24".parse().unwrap(),
            gateway_ipv4: "172.31.1.1".parse().unwrap(),
            router_ipv4: "172.31.1.2".parse().unwrap(),
            ipv6_management_subnet: "fd00:b00b:0:1::/64".parse().unwrap(),
            ipv6_loopback_subnet: "fd00:b00c:0:1::/64".parse().unwrap(),
            gateway_ipv6: "fd00:b00b:0:1::1".parse().unwrap(),
            router_ipv6: "fd00:b00b:0:1::2".parse().unwrap(),
        }
    }

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn boot_server_010_its_own_folder_holds_only_its_config() {
        let dir = tempfile::tempdir().unwrap();

        write_dnsmasq_config(
            dir.path().to_str().unwrap(),
            &management_network(&subnets()).unwrap(),
            &[],
            &[],
        )
        .unwrap();

        assert_eq!(file_names(dir.path()), [DNSMASQ_CONFIG_FILE]);
    }

    /// The lab's nodes, by name and model.
    const MODELS: [(&str, NodeModel); 3] = [
        ("ctr", NodeModel::PostgresqlDb),
        ("vm", NodeModel::AlpineLinux),
        ("uk", NodeModel::NanosUnikernel),
    ];

    fn test_config() -> data::Config {
        toml::from_str(
            r#"
            name = "test"
            vm_provider = "libvirt"
            qemu_bin = "/usr/bin/qemu-system-x86_64"
            images_dir = "/opt/sherpa/images"
            containers_dir = "/opt/sherpa/containers"
            bins_dir = "/opt/sherpa/bins"
            "#,
        )
        .unwrap()
    }

    /// Each of `MODELS`' setup data and node data, from its record in `records`, with the
    /// resources phase 6 reserves for it.
    fn node_data(records: &[data::DbNode]) -> (Vec<data::NodeSetupData>, Vec<data::LabNodeData>) {
        MODELS
            .iter()
            .map(|(name, model)| {
                let record = records.iter().find(|record| record.name == *name).unwrap();
                let setup = data::NodeSetupData {
                    name: (*name).to_owned(),
                    index: record.index,
                    management_network: "sherpa-management-lab-0001".to_owned(),
                    isolated_network: None,
                    reserved_network: None,
                    interfaces: Vec::new(),
                };
                let node = data::LabNodeData {
                    name: (*name).to_owned(),
                    model_kind: ModelKind::default_for(*model),
                    index: record.index,
                    record: data::DbNode {
                        cpu_count: Some(1),
                        memory: Some(1024),
                        ..record.clone()
                    },
                };
                (setup, node)
            })
            .unzip()
    }

    /// The nodes of `MODELS` of `kind`, as the manifest gives them.
    fn nodes_of(kind: NodeKind) -> Vec<topology::NodeExpanded> {
        MODELS
            .iter()
            .filter(|(_, model)| ModelKind::default_for(*model).kind() == kind)
            .map(|(name, model)| topology::NodeExpanded {
                name: (*name).to_owned(),
                model_kind: ModelKind::default_for(*model),
                ..Default::default()
            })
            .collect()
    }

    #[tokio::test]
    #[ignore = "requires libvirt: phase 8 takes a connection, though it creates nothing in it"]
    async fn config_delivery_028_every_container_and_vm_gets_a_certificate_and_a_unikernel_none() {
        let (db, lab) = lab_with(&MODELS).await;
        let records = db::list_nodes_by_lab(&db, lab.id.clone().unwrap())
            .await
            .unwrap();
        let (node_setup_data, lab_node_data) = node_data(&records);
        let node_images: Vec<data::NodeConfig> = MODELS
            .iter()
            .map(|(_, model)| data::NodeConfig {
                default: true,
                ..data::ModelKind::default_for(*model).node_config()
            })
            .collect();
        let lab_dir = tempfile::tempdir().unwrap();
        let qemu = Arc::new(Qemu::default());
        let (tx, _rx) = mpsc::unbounded_channel();
        let build = LabBuild {
            lab_id: &lab.lab_id,
            lab_dir: lab_dir.path().to_str().unwrap(),
            db: &db,
            docker_conn: &Arc::new(Docker::connect_with_unix_defaults().unwrap()),
            qemu_conn: &Arc::new(qemu.connect().unwrap()),
            qemu: &qemu,
            config: &test_config(),
            sandbox_disk_locks: &SandboxDiskLocks::default(),
            progress: &ProgressSender::new(tx),
        };
        let mgmt_net = management_network(&subnets()).unwrap();

        configure_nodes(
            &build,
            &ConfigInputs {
                lab_record_id: lab.id.as_ref().unwrap(),
                node_images: &node_images,
                node_setup_data: &node_setup_data,
                lab_node_data: &lab_node_data,
                mgmt_net: &mgmt_net,
                dns: &util::default_dns(&mgmt_net),
                owner_account: &test_account("alice", &[]),
                talos_clusters: &[],
            },
            &mut nodes_of(NodeKind::Container),
            &mut nodes_of(NodeKind::VirtualMachine),
            &mut nodes_of(NodeKind::Unikernel),
        )
        .await
        .unwrap();

        assert_eq!(
            file_names(&lab_dir.path().join(LAB_CERTS_DIR)),
            [
                LAB_CA_CERT_FILE,
                LAB_CA_KEY_FILE,
                "ctr.crt",
                "ctr.key",
                "vm.crt",
                "vm.key"
            ]
        );
    }
}
