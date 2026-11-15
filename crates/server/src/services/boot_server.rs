//! The lab's boot server: the dnsmasq config its container is started with, and, when a
//! redeploy changes a node's boot options, the config written again and the container restarted.
//!
//! dnsmasq reads its main config only when it starts; `SIGHUP` rereads its host files, not
//! `dnsmasq.conf`. So a changed config takes a restart of the lab's dnsmasq container. The
//! leases stay: they are kept inside the container.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result};
use askama::Template;
use bollard::Docker;
use tracing::instrument;

use db::Database;
use shared::data::{self, DbNode, NodeConfig, NodeKind, RecordId, Ztp, ZtpRecord};
use shared::konst::{
    CONTAINER_DNSMASQ_NAME, DNSMASQ_CONFIG_FILE, DNSMASQ_DIR, MANAGEMENT_NODE_HOST_OFFSETS,
    SSH_PORT, ZTP_DIR,
};
use shared::util;
use shared::validators::effective_ztp;

/// A node of a lab as its boot server sees it: its record, its image, and the ZTP method its
/// definition sets, if any.
pub(crate) struct BootNode {
    pub(crate) record: DbNode,
    pub(crate) image: NodeConfig,
    pub(crate) method: Option<Ztp>,
}

/// The lab's dnsmasq config: DHCP and DNS for the management network `mgmt_net`, and a DHCP
/// host entry, with its boot options, for each of `ztp_records`.
///
/// # Errors
///
/// Fails when an address of the DHCP range cannot be made, or the template does not render.
pub(crate) fn render_dnsmasq_config(
    mgmt_net: &data::SherpaNetwork,
    ztp_records: &[ZtpRecord],
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
    };
    Ok(template.render()?)
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
    ordered.sort_by_key(|node| (kind_order(&node.image.kind), node.record.index));
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
        records.push(match node.image.kind {
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
                ssh_port: node.image.model.has_ssh().then_some(SSH_PORT),
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
fn kind_order(kind: &NodeKind) -> u8 {
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
/// configured again: render the dnsmasq config from the lab's nodes, with the redeployed node's
/// new record, and when it differs from the one in the lab folder `lab_dir` (the node's boot
/// options changed with its ZTP method) write it and restart the lab's dnsmasq container.
/// Returns whether it restarted.
///
/// A node's method is its definition's, else its image's. A lab with no stored definition
/// gives every other node its image's.
///
/// # Errors
///
/// Fails when the nodes cannot be read, the config cannot be written, or the container cannot
/// be restarted.
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
    let config = render_dnsmasq_config(mgmt_net, &lab_ztp_records(&nodes, redeployed)?)?;
    let path = format!("{lab_dir}/{ZTP_DIR}/{DNSMASQ_DIR}/{DNSMASQ_CONFIG_FILE}");
    if !rewrite_dnsmasq_config(&path, &config)? {
        return Ok(false);
    }
    let container_name = format!("{CONTAINER_DNSMASQ_NAME}-{lab_id}");
    container::restart_container(docker, &container_name)
        .await
        .context("Failed to restart the lab's boot server with its new config")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use shared::data::NodeModel;

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
                vnc_port: None,
                ssh_host_key: None,
                ready_check: data::ReadyCheck::default(),
                ssh_username: None,
            },
            image: NodeConfig::get_model(model),
            method,
        }
    }

    /// A lab of a vSRX `srx`, a `SONiC` switch `sonic`, a container and a unikernel.
    fn lab() -> Vec<BootNode> {
        vec![
            node("uk1", 1, NodeModel::NanosUnikernel, None),
            node("srx", 2, NodeModel::JuniperVsrxv3, Some(Ztp::Cdrom)),
            node("ct1", 3, NodeModel::GenericContainer, None),
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
        render_dnsmasq_config(&mgmt_net(), &lab_ztp_records(&lab(), redeployed).unwrap()).unwrap()
    }

    const SRX_TFTP: &str = "dhcp-option=tag:srx-tag,67,srx.conf";
    const SONIC_HTTP: &str =
        r#"dhcp-option=tag:sonic-tag,67,"http://172.31.1.2:8080/configs/sonic.conf""#;

    #[test]
    fn a_redeploy_onto_tftp_gives_the_node_its_boot_option() {
        let before = config(&redeployed("srx", 2, Ztp::Cdrom));
        let after = config(&redeployed("srx", 2, Ztp::Tftp));

        assert!(!before.contains(SRX_TFTP), "{before}");
        assert!(after.contains(SRX_TFTP), "{after}");
        // The other nodes' entries are as they were.
        assert!(before.contains(SONIC_HTTP) && after.contains(SONIC_HTTP));
    }

    #[test]
    fn a_redeploy_off_tftp_removes_the_nodes_boot_option() {
        let after = config(&redeployed("srx", 2, Ztp::Cdrom));

        assert!(!after.contains("srx-tag,67"), "{after}");
        assert!(after.contains("dhcp-host=52:54:00:00:00:02,172.31.1.22,srx,,set:srx-tag"));
    }

    #[test]
    fn a_redeploy_onto_and_off_http_adds_and_removes_the_nodes_boot_options() {
        let on = config(&redeployed("sonic", 4, Ztp::Http));
        let off = config(&redeployed("sonic", 4, Ztp::Disabled));

        assert!(on.contains(SONIC_HTTP), "{on}");
        assert!(
            on.contains(
                r#"dhcp-option=tag:sonic-tag,239,"http://172.31.1.2:8080/configs/sonic.conf""#
            ),
            "{on}"
        );
        assert!(
            !off.contains("sonic-tag,67") && !off.contains("sonic-tag,239"),
            "{off}"
        );
    }

    #[test]
    fn the_records_are_in_the_order_up_makes_them() {
        let records = lab_ztp_records(&lab(), &redeployed("srx", 2, Ztp::Cdrom)).unwrap();

        let names: Vec<&str> = records.iter().map(|r| r.node_name.as_str()).collect();
        assert_eq!(names, ["ct1", "srx", "sonic", "uk1"]);
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
    fn a_node_without_an_address_has_no_entry() {
        let mut nodes = lab();
        nodes[2].record.mgmt_ipv4 = None;

        let records = lab_ztp_records(&nodes, &redeployed("srx", 2, Ztp::Tftp)).unwrap();

        assert!(records.iter().all(|record| record.node_name != "ct1"));
    }

    #[test]
    fn the_config_is_written_only_when_it_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DNSMASQ_CONFIG_FILE);
        let path = path.to_str().unwrap();

        assert!(rewrite_dnsmasq_config(path, "port=53\n").unwrap());
        assert!(!rewrite_dnsmasq_config(path, "port=53\n").unwrap());
        assert!(rewrite_dnsmasq_config(path, "port=54\n").unwrap());
        assert_eq!(fs::read_to_string(path).unwrap(), "port=54\n");
    }
}
