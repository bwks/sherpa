//! The lab's boot server: its container and the dnsmasq config it is started with, and, when a
//! redeploy changes the config, the config written again and the container created again.
//!
//! The container runs dnsmasq for DHCP, DNS and TFTP, and an HTTP server for the nodes that boot
//! by HTTP. The image's HTTP server serves everything under its root, so only the lab's HTTP
//! configs folder is mounted there: the TFTP folder is mounted outside it, and the leases are
//! kept inside the container, outside it too.
//!
//! dnsmasq reads its main config only when it starts; `SIGHUP` rereads its host files, not
//! `dnsmasq.conf`. So a changed config takes a new container. It is created again rather than
//! restarted, so a lab made by an older release, whose container still has the TFTP folder in
//! its HTTP root, gets the current mounts with the config that expects them. The leases start
//! again empty; every node's address is reserved by its MAC, so none changes.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result, ensure};
use askama::Template;
use bollard::Docker;
use tracing::instrument;

use container::ContainerSpec;
use db::Database;
use shared::data::{
    self, ContainerNetworkAttachment, DbNode, NodeConfig, NodeKind, RecordId, Ztp, ZtpRecord,
};
use shared::konst::{
    BOOT_SERVER_HTTP_ROOT, BOOT_SERVER_TFTP_ROOT, CONTAINER_DNSMASQ_NAME, CONTAINER_DNSMASQ_REPO,
    DNSMASQ_DIR, MANAGEMENT_NODE_HOST_OFFSETS, NODE_CONFIGS_DIR, SSH_PORT, TFTP_DIR, ZTP_DIR,
};
use shared::util;
use shared::validators::effective_ztp;

use crate::konst::{DNSMASQ_CONFIG_FILE, SHERPA_MANAGEMENT_NETWORK_NAME};

/// The capabilities the boot container needs: dnsmasq's DHCP, and the image's NAT rules.
const BOOT_CONTAINER_CAPABILITIES: &[&str] = &["NET_ADMIN"];

/// A node of a lab as its boot server sees it: its record, its image, and the ZTP method its
/// definition sets, if any.
pub(crate) struct BootNode {
    pub(crate) record: DbNode,
    pub(crate) image: NodeConfig,
    pub(crate) method: Option<Ztp>,
}

/// The lab's dnsmasq config: DHCP and DNS for the management network `mgmt_net`, a DHCP host
/// entry, with its boot options, for each of `ztp_records`, and the names and reservations of
/// `sandbox_hosts`.
///
/// # Errors
///
/// Fails when an address of the DHCP range cannot be made, or the template does not render.
pub(crate) fn render_dnsmasq_config(
    mgmt_net: &data::SherpaNetwork,
    ztp_records: &[ZtpRecord],
    sandbox_hosts: &[template::SandboxHostRecord],
) -> Result<String> {
    let template = template::DnsmasqTemplate {
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
        sandbox_hosts: sandbox_hosts.to_vec(),
    };
    Ok(template.render()?)
}

/// The boot server's names and DHCP reservation for the own addresses of the sandbox of node
/// `node_name`.
pub(crate) fn sandbox_host_record(
    node_name: &str,
    address: &data::DbSandboxAddress,
) -> template::SandboxHostRecord {
    template::SandboxHostRecord {
        node_name: node_name.to_owned(),
        ipv4: address.ipv4,
        ipv6: address.ipv6,
        mac: address.mac.clone(),
    }
}

/// The DHCP host records of a lab's `nodes`, as `up` makes them: containers, then VMs, then
/// unikernels, each in index order. `redeployed`, the record a redeploy just made, replaces its
/// node's. A node with no management address has no record; it is logged.
///
/// # Errors
///
/// Fails when a node's stored address is not an address.
pub(crate) fn lab_ztp_records(
    nodes: &[BootNode],
    redeployed: &ZtpRecord,
) -> Result<Vec<ZtpRecord>> {
    let mut ordered: Vec<&BootNode> = nodes.iter().collect();
    ordered.sort_by_key(|node| (kind_order(node.image.kind()), node.record.index));
    let mut records = Vec::with_capacity(ordered.len());
    for node in ordered {
        let name = &node.record.name;
        if *name == redeployed.node_name {
            records.push(redeployed.clone());
            continue;
        }
        let Some(ipv4) = node.record.mgmt_ipv4.as_deref() else {
            tracing::warn!(node_name = %name, "The node has no management address; it has no DHCP host entry");
            continue;
        };
        let ipv4: Ipv4Addr = ipv4
            .parse()
            .with_context(|| format!("Node '{name}' has an invalid management address"))?;
        let mac_address = node.record.mgmt_mac.clone().unwrap_or_default();
        records.push(match node.image.kind() {
            NodeKind::Container => ZtpRecord {
                node_name: name.clone(),
                config_file: format!("{name}.conf"),
                ipv4_address: ipv4,
                ipv6_address: None,
                mac_address: String::new(),
                ztp: effective_ztp(node.method, &node.image),
                ssh_port: Some(SSH_PORT),
            },
            NodeKind::VirtualMachine => ZtpRecord {
                node_name: name.clone(),
                config_file: format!("{name}.conf"),
                ipv4_address: ipv4,
                ipv6_address: None,
                mac_address,
                ztp: effective_ztp(node.method, &node.image),
                ssh_port: node.image.model_kind()?.has_ssh().then_some(SSH_PORT),
            },
            NodeKind::Unikernel => ZtpRecord {
                node_name: name.clone(),
                config_file: String::new(),
                ipv4_address: ipv4,
                ipv6_address: node
                    .record
                    .mgmt_ipv6
                    .as_deref()
                    .map(str::parse::<Ipv6Addr>)
                    .transpose()
                    .with_context(|| format!("Node '{name}' has an invalid IPv6 address"))?,
                mac_address,
                ztp: Ztp::Disabled,
                ssh_port: Some(SSH_PORT),
            },
        });
    }
    Ok(records)
}

/// Where `up` puts each kind of node among the boot server's records.
fn kind_order(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::Container => 0,
        NodeKind::VirtualMachine => 1,
        NodeKind::Unikernel => 2,
    }
}

/// Write `config` to `path` unless the file there holds it already. Returns whether it wrote.
///
/// # Errors
///
/// Fails when the file cannot be read or written.
#[instrument(skip(config), level = "debug")]
pub(crate) fn rewrite_dnsmasq_config(path: &str, config: &str) -> Result<bool> {
    match fs::read_to_string(path) {
        Ok(current) if current == config => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("Failed to read {path}")),
    }
    util::create_file(path, config.to_owned())?;
    Ok(true)
}

/// Bring the boot server of lab `lab_id` up to date after its node `redeployed.node_name` was
/// configured again: render the dnsmasq config from the lab's nodes and sandbox addresses, with
/// the redeployed node's new record, and when it differs from the one in the lab folder `lab_dir` (the node's boot
/// options changed with its ZTP method, or the lab was made by an older release) write it and
/// create the lab's boot container again. Returns whether it was recreated.
///
/// A node's method is its definition's, else its image's. A lab with no stored definition
/// gives every other node its image's.
///
/// # Errors
///
/// Fails when the nodes cannot be read, the config cannot be written, or the container cannot
/// be removed or started again.
#[instrument(skip(db, docker, mgmt_net, redeployed), fields(%lab_id, node_name = %redeployed.node_name))]
pub(crate) async fn refresh_boot_server(
    db: &Database,
    docker: &Docker,
    lab_id: &str,
    lab_record_id: &RecordId,
    lab_dir: &str,
    mgmt_net: &data::SherpaNetwork,
    redeployed: &ZtpRecord,
) -> Result<bool> {
    let records = db::list_nodes_by_lab(db, lab_record_id.clone())
        .await
        .context("Failed to list the lab's nodes")?;
    let images: HashMap<RecordId, NodeConfig> =
        db::list_node_images_by_ids(db, records.iter().map(|node| node.image.clone()).collect())
            .await
            .context("Failed to read the images of the lab's nodes")?
            .into_iter()
            .filter_map(|image| image.id.clone().map(|id| (id, image)))
            .collect();
    let methods: HashMap<String, Option<Ztp>> = db::read_lab_definition(db, lab_record_id)
        .await
        .context("Failed to read the lab's definition")?
        .map_or_default(|manifest| {
            manifest
                .nodes
                .into_iter()
                .map(|node| (node.name, node.ztp.method))
                .collect()
        });
    let mut nodes = Vec::with_capacity(records.len());
    for record in records {
        let image = images
            .get(&record.image)
            .with_context(|| format!("Node '{}' has no image record", record.name))?
            .clone();
        let method = methods.get(&record.name).copied().flatten();
        nodes.push(BootNode {
            record,
            image,
            method,
        });
    }
    let names: HashMap<&RecordId, &str> = nodes
        .iter()
        .filter_map(|node| Some((node.record.id.as_ref()?, node.record.name.as_str())))
        .collect();
    let sandbox_hosts: Vec<template::SandboxHostRecord> =
        db::list_sandbox_addresses(db, lab_record_id)
            .await
            .context("Failed to list the lab's sandbox addresses")?
            .iter()
            .filter_map(|address| Some(sandbox_host_record(names.get(&address.node)?, address)))
            .collect();
    let config = render_dnsmasq_config(
        mgmt_net,
        &lab_ztp_records(&nodes, redeployed)?,
        &sandbox_hosts,
    )?;
    let path = format!("{lab_dir}/{ZTP_DIR}/{DNSMASQ_DIR}/{DNSMASQ_CONFIG_FILE}");
    if !rewrite_dnsmasq_config(&path, &config)? {
        return Ok(false);
    }
    container::remove_container(docker, &format!("{CONTAINER_DNSMASQ_NAME}-{lab_id}"))
        .await
        .context("Failed to remove the lab's boot server to start it with its new config")?;
    run_boot_container(docker, lab_id, &format!("{lab_dir}/{ZTP_DIR}"), mgmt_net)
        .await
        .context("Failed to start the lab's boot server with its new config")?;
    Ok(true)
}

/// The boot container of lab `lab_id`, on its management network `mgmt_net`, serving the lab's
/// ZTP folder `ztp_dir`: the HTTP configs folder as the HTTP root and nothing else under it, the
/// TFTP folder as the TFTP root, and the dnsmasq config.
fn boot_container_spec(
    lab_id: &str,
    ztp_dir: &str,
    mgmt_net: &data::SherpaNetwork,
) -> ContainerSpec {
    ContainerSpec {
        name: format!("{CONTAINER_DNSMASQ_NAME}-{lab_id}"),
        hostname: None,
        image: CONTAINER_DNSMASQ_REPO.to_owned(),
        env_vars: vec![format!("DNS1={}", mgmt_net.v4.first), "DNS2=".to_owned()],
        volumes: vec![
            data::ContainerMount::writable(
                format!("{ztp_dir}/{DNSMASQ_DIR}/{DNSMASQ_CONFIG_FILE}"),
                format!("/etc/{DNSMASQ_CONFIG_FILE}"),
            ),
            data::ContainerMount::writable(format!("{ztp_dir}/{TFTP_DIR}"), BOOT_SERVER_TFTP_ROOT),
            data::ContainerMount::writable(
                format!("{ztp_dir}/{NODE_CONFIGS_DIR}"),
                BOOT_SERVER_HTTP_ROOT,
            ),
        ],
        capabilities: BOOT_CONTAINER_CAPABILITIES
            .iter()
            .map(ToString::to_string)
            .collect(),
        management_network: ContainerNetworkAttachment {
            name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            ipv4_address: Some(mgmt_net.v4.boot_server.to_string()),
            ipv6_address: None,
            linux_interface_name: None,
            admin_down: false,
        },
        additional_networks: vec![],
        commands: vec![],
        privileged: false,
        shm_size: None,
        user: None,
        memory_mib: None,
        cpu_count: None,
    }
}

/// Create and start the boot container of lab `lab_id` (see [`boot_container_spec`]). The
/// lab's ZTP folder `ztp_dir` must hold its TFTP and HTTP configs folders and its dnsmasq
/// config.
///
/// # Errors
///
/// Fails when Docker cannot create or start the container, or it is not running afterwards.
#[instrument(skip(docker, mgmt_net), fields(%lab_id))]
pub(crate) async fn run_boot_container(
    docker: &Docker,
    lab_id: &str,
    ztp_dir: &str,
    mgmt_net: &data::SherpaNetwork,
) -> Result<()> {
    tracing::info!(
        lab_id = %lab_id,
        boot_server_ip = %mgmt_net.v4.boot_server,
        "Starting the lab's boot container"
    );
    let is_running =
        container::run_container(docker, boot_container_spec(lab_id, ztp_dir, mgmt_net)).await?;
    ensure!(
        is_running,
        "Boot container {CONTAINER_DNSMASQ_NAME}-{lab_id} is not running after start"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use shared::data::{ModelKind, NodeModel};

    use super::*;

    fn mgmt_net() -> data::SherpaNetwork {
        data::SherpaNetwork {
            v4: data::NetworkV4 {
                prefix: "172.31.1.0/24".parse().unwrap(),
                first: "172.31.1.1".parse().unwrap(),
                last: "172.31.1.255".parse().unwrap(),
                boot_server: "172.31.1.2".parse().unwrap(),
                network: "172.31.1.0".parse().unwrap(),
                subnet_mask: "255.255.255.0".parse().unwrap(),
                hostmask: "0.0.0.255".parse().unwrap(),
                prefix_length: 24,
            },
            v6: None,
        }
    }

    fn node(name: &str, index: u16, model: NodeModel, method: Option<Ztp>) -> BootNode {
        BootNode {
            record: DbNode {
                id: None,
                name: name.to_owned(),
                image: RecordId::new("node_image", name),
                index,
                lab: RecordId::new("lab", "lab"),
                mgmt_ipv4: Some(format!("172.31.1.{}", 20 + index)),
                mgmt_ipv6: Some(format!("fd00:b00b:0:1::{}", 20 + index)),
                mgmt_mac: Some(format!("52:54:00:00:00:{index:02x}")),
                state: data::NodeState::Running,
                cpu_count: None,
                memory: None,
                ssh_host_key: None,
                ready_check: data::ReadyCheck::default(),
                ssh_username: None,
            },
            image: ModelKind::default_for(model).node_config(),
            method,
        }
    }

    /// A lab of a Junos Evolved VM `evo`, a `SONiC` switch `sonic`, a container and a unikernel.
    fn lab() -> Vec<BootNode> {
        vec![
            node("uk1", 1, NodeModel::NanosUnikernel, None),
            node("evo", 2, NodeModel::JuniperEvolved, Some(Ztp::Usb)),
            node("ct1", 3, NodeModel::PostgresqlDb, None),
            node("sonic", 4, NodeModel::SonicLinux, Some(Ztp::Http)),
        ]
    }

    /// The record a redeploy makes for `name`, the `index`th node, booting by `method`.
    fn redeployed(name: &str, index: u16, method: Ztp) -> ZtpRecord {
        ZtpRecord {
            node_name: name.to_owned(),
            config_file: format!("{name}.conf"),
            ipv4_address: format!("172.31.1.{}", 20 + index).parse().unwrap(),
            ipv6_address: None,
            mac_address: format!("52:54:00:00:00:{index:02x}"),
            ztp: method,
            ssh_port: Some(SSH_PORT),
        }
    }

    fn config(redeployed: &ZtpRecord) -> String {
        render_dnsmasq_config(
            &mgmt_net(),
            &lab_ztp_records(&lab(), redeployed).unwrap(),
            &[],
        )
        .unwrap()
    }

    const EVO_TFTP: &str = "dhcp-option=tag:evo-tag,67,evo.conf";
    const SONIC_HTTP: &str = r#"dhcp-option=tag:sonic-tag,67,"http://172.31.1.2:8080/sonic.conf""#;

    #[test]
    fn boot_server_011_a_redeploy_onto_tftp_gives_the_node_its_boot_option() {
        let before = config(&redeployed("evo", 2, Ztp::Usb));
        let after = config(&redeployed("evo", 2, Ztp::Tftp));

        assert!(!before.contains(EVO_TFTP), "{before}");
        assert!(after.contains(EVO_TFTP), "{after}");
        // The other nodes' entries are as they were.
        assert!(before.contains(SONIC_HTTP) && after.contains(SONIC_HTTP));
    }

    #[test]
    fn boot_server_012_a_redeploy_off_tftp_removes_the_boot_option_and_keeps_the_lease() {
        let after = config(&redeployed("evo", 2, Ztp::Usb));

        assert!(!after.contains("evo-tag,67"), "{after}");
        assert!(after.contains("dhcp-host=52:54:00:00:00:02,172.31.1.22,evo,,set:evo-tag"));
    }

    #[test]
    fn boot_server_013_a_redeploy_onto_and_off_http_adds_and_removes_both_options() {
        let on = config(&redeployed("sonic", 4, Ztp::Http));
        let off = config(&redeployed("sonic", 4, Ztp::Disabled));

        assert!(on.contains(SONIC_HTTP), "{on}");
        assert!(
            on.contains(r#"dhcp-option=tag:sonic-tag,239,"http://172.31.1.2:8080/sonic.conf""#),
            "{on}"
        );
        assert!(
            !off.contains("sonic-tag,67") && !off.contains("sonic-tag,239"),
            "{off}"
        );
    }

    #[test]
    fn boot_server_014_entries_after_a_redeploy_are_in_the_order_up_wrote_them() {
        let records = lab_ztp_records(&lab(), &redeployed("evo", 2, Ztp::Usb)).unwrap();

        let names: Vec<&str> = records.iter().map(|r| r.node_name.as_str()).collect();
        assert_eq!(names, ["ct1", "evo", "sonic", "uk1"]);
        // As `up` makes them: no MAC for a container, the IPv6 address for a unikernel only.
        assert_eq!(records[0].mac_address, "");
        assert_eq!(records[2].ipv6_address, None);
        assert_eq!(
            records[3].ipv6_address,
            Some("fd00:b00b:0:1::21".parse().unwrap())
        );
        assert_eq!(records[3].ztp, Ztp::Disabled);
    }

    #[test]
    fn boot_server_015_a_node_without_an_address_has_no_entry() {
        let mut nodes = lab();
        nodes[2].record.mgmt_ipv4 = None;

        let records = lab_ztp_records(&nodes, &redeployed("evo", 2, Ztp::Tftp)).unwrap();

        assert!(records.iter().all(|record| record.node_name != "ct1"));
    }

    #[test]
    fn boot_server_016_the_config_is_written_only_when_it_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DNSMASQ_CONFIG_FILE);
        let path = path.to_str().unwrap();

        assert!(rewrite_dnsmasq_config(path, "port=53\n").unwrap());
        assert!(!rewrite_dnsmasq_config(path, "port=53\n").unwrap());
        assert!(rewrite_dnsmasq_config(path, "port=54\n").unwrap());
        assert_eq!(fs::read_to_string(path).unwrap(), "port=54\n");
    }

    /// Whether the container path `path` is the boot container's HTTP root or under it.
    fn served_over_http(path: &str) -> bool {
        path == BOOT_SERVER_HTTP_ROOT || path.starts_with(&format!("{BOOT_SERVER_HTTP_ROOT}/"))
    }

    #[test]
    fn boot_server_008_only_the_http_configs_folder_is_served_over_http() {
        let spec = boot_container_spec("abcd1234", "/labs/abcd1234/ztp", &mgmt_net());

        let served: Vec<&str> = spec
            .volumes
            .iter()
            .filter(|volume| served_over_http(&volume.destination))
            .map(|volume| volume.source.as_str())
            .collect();

        assert_eq!(served, ["/labs/abcd1234/ztp/configs"]);
    }

    #[test]
    fn boot_server_009_the_tftp_root_and_leases_are_outside_the_http_root() {
        let config = render_dnsmasq_config(&mgmt_net(), &[], &[]).unwrap();
        let value = |key: &str| {
            config
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .unwrap()
                .to_owned()
        };

        let tftp_root = value("tftp-root=");
        let leases = value("dhcp-leasefile=");

        assert!(!served_over_http(&tftp_root));
        assert!(!served_over_http(&leases));
    }
}
