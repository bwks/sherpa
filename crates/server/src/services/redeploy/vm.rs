//! Stages 4 to 6 for a VM: its configuration, disks, interfaces and domain, and waiting for it
//! to become ready.

use std::collections::HashMap;
use std::mem;
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::instrument;

use super::context::Redeploy;
use crate::services::node_lifecycle;
use crate::services::node_ops::{self, DomainParts, LabZtp, VmReadiness, VmZtpResult};
use crate::services::node_resources;
use crate::services::p2p_links::{
    KernelHost, LabLinks, RecreatedEnd, data_interface_indices, node_bridge_interfaces, node_links,
    reattach_node_links, report_links,
};
use crate::tls::lab_ca::NodeCertPaths;

use shared::data;
use shared::data::{DbLink, LinkReport, OperationEvent, RecordId, StatusKind};
use shared::konst::{
    KVM_OUI, MTU_JUMBO_NET, READINESS_SLEEP, SHERPA_MANAGEMENT_NETWORK_NAME, TFTP_DIR, ZTP_DIR,
};
use shared::util;

/// Configure the VM again, clone its disks, create its domain on its links, bridges and
/// networks, re-attach its links, and wait for it to become ready (stages 4 to 6). Returns its
/// links' states.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
pub(super) async fn redeploy_vm(
    redeploy: &mut Redeploy<'_>,
    cert_paths: &NodeCertPaths,
) -> Result<Vec<LinkReport>> {
    let vm_ztp = configure_vm(redeploy, cert_paths).await?;
    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let progress = redeploy.progress;

    let db = &redeploy.state.db;
    let lab_links = LabLinks::load(db, &redeploy.lab_record_id).await?;
    let links = node_links(lab_links.links(), &redeploy.node_record_id);
    let db_bridges = db::list_bridges(db, &redeploy.lab_record_id).await?;
    let interfaces = vm_interfaces(
        &redeploy.node_image,
        &redeploy.node_record_id,
        &vm_ztp.mac_address,
        data_interface_indices(
            &redeploy.node_image,
            redeploy.data_interface_count,
            node_name,
        )?,
        &links,
        &node_bridge_interfaces(&db_bridges, &redeploy.node_record_id),
    )?;

    let node_idx = redeploy.db_node.index;
    let loopback_host = u8::try_from(node_idx)
        .with_context(|| format!("Node '{node_name}' has an index past the loopback network"))?;
    let isolated_net = node_ops::node_isolated_network_data(node_name, node_idx, lab_id);
    let reserved_network = if redeploy.node_image.interfaces.reserved_count > 0 {
        node_ops::node_reserved_network_data(node_name, node_idx, lab_id).network_name
    } else {
        String::new()
    };
    let has_disabled = interfaces
        .iter()
        .any(|i| matches!(i.connection_type, data::ConnectionTypes::Disabled));

    let domain = node_ops::build_domain_template(
        &redeploy.target_node,
        &redeploy.node_image,
        lab_id,
        &redeploy.state.config.qemu_bin,
        DomainParts {
            disks: vm_ztp.disks,
            interfaces,
            loopback_ipv4: util::get_ip(&redeploy.lab_info.loopback_network, loopback_host)
                .to_string(),
            management_network: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            isolated_network: isolated_net.network_name,
            reserved_network,
        },
        vm_ztp.qemu_commands,
        &node_resources::resources_of(&redeploy.db_node),
    )?;

    let _ = progress.send_status(format!("Creating VM: {node_name}"), StatusKind::Progress);
    node_ops::create_vm(redeploy.qemu_conn.clone(), domain, progress).await?;
    node_lifecycle::record(
        &redeploy.state.db,
        redeploy.operation,
        &redeploy.node_record_id,
        OperationEvent::Launched,
    )
    .await?;

    let outcomes = reattach_node_links(
        &KernelHost::new(&redeploy.state.docker),
        &lab_links,
        &redeploy.node_record_id,
        RecreatedEnd::Domain,
    )
    .await;
    let link_reports = report_links(
        Some(progress),
        &redeploy.state.link_states,
        lab_id,
        node_name,
        outcomes,
    );

    // Set isolated bridge DOWN to remove carrier from disabled VM interfaces
    if has_disabled {
        network::set_link_down(&isolated_net.bridge_name).await?;
    }

    wait_until_ready(redeploy).await?;
    Ok(link_reports)
}

/// Generate the VM's configuration again and clone its disks (stage 4). The returned result's
/// disks to clone are taken: they are cloned.
async fn configure_vm(
    redeploy: &mut Redeploy<'_>,
    cert_paths: &NodeCertPaths,
) -> Result<VmZtpResult> {
    let ztp_dir = format!("{}/{ZTP_DIR}", redeploy.lab_dir);
    util::create_dir(&ztp_dir)?;
    util::create_dir(&format!("{ztp_dir}/{TFTP_DIR}"))?;

    let mut vm_ztp = node_ops::generate_vm_ztp(
        &mut redeploy.target_node,
        &redeploy.node_image,
        &LabZtp {
            lab_id: &redeploy.lab_id,
            lab_dir: &redeploy.lab_dir,
            images_dir: &redeploy.state.config.images_dir,
            sherpa_user: &redeploy.sherpa_user,
            dns: &redeploy.dns,
            mgmt_net: &redeploy.mgmt_net,
            progress: redeploy.progress,
            owner_account: redeploy.owner_account.as_ref(),
        },
        redeploy.ipv4,
        redeploy.db_node.mgmt_mac.as_deref(),
        Some(cert_paths),
    )?;

    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let progress = redeploy.progress;
    let _ = progress.send_status(
        format!("ZTP regenerated for node: {node_name}"),
        StatusKind::Done,
    );

    let _ = progress.send_status(
        format!("Cloning disks for node: {node_name}"),
        StatusKind::Progress,
    );
    node_ops::clone_node_disks(
        redeploy.qemu_conn.clone(),
        mem::take(&mut vm_ztp.clone_disks),
        lab_id,
        progress,
        vec![redeploy.image_guard.clone()],
    )
    .await?;
    Ok(vm_ztp)
}

/// The VM's interfaces: management, reserved, then each data interface on its P2p link, on
/// its private bridge, or, disabled, on the node's isolated network.
fn vm_interfaces(
    node_image: &data::NodeConfig,
    node: &RecordId,
    mgmt_mac: &str,
    data_indices: RangeInclusive<u8>,
    links: &[&DbLink],
    bridge_interfaces: &HashMap<String, String>,
) -> Result<Vec<data::Interface>> {
    let mut interfaces = vec![data::Interface {
        name: util::dasher(&node_image.interfaces.management.to_string()),
        num: 0,
        mtu: node_image.interfaces.mtu,
        mac_address: mgmt_mac.to_owned(),
        connection_type: data::ConnectionTypes::Management,
    }];

    for idx in 1..*data_indices.start() {
        interfaces.push(data::Interface {
            name: format!("int{idx}"),
            num: idx,
            mtu: node_image.interfaces.mtu,
            mac_address: util::random_mac(KVM_OUI),
            connection_type: data::ConnectionTypes::Reserved,
        });
    }

    for idx in data_indices {
        let interface_name = util::interface_from_idx(&node_image.model, idx)?;
        let tap = links.iter().find_map(|link| {
            if link.node_a == *node && link.int_a == interface_name {
                Some(&link.tap_a)
            } else if link.node_b == *node && link.int_b == interface_name {
                Some(&link.tap_b)
            } else {
                None
            }
        });
        let (name, mtu, connection_type) = match (tap, bridge_interfaces.get(&interface_name)) {
            (Some(tap), _) => (tap.clone(), MTU_JUMBO_NET, data::ConnectionTypes::P2p),
            (None, Some(bridge_name)) => (
                bridge_name.clone(),
                node_image.interfaces.mtu,
                data::ConnectionTypes::PrivateBridge,
            ),
            // Disabled: on the node's isolated network.
            (None, None) => (
                util::dasher(&interface_name),
                node_image.interfaces.mtu,
                data::ConnectionTypes::Disabled,
            ),
        };
        interfaces.push(data::Interface {
            name,
            num: idx,
            mtu,
            mac_address: util::random_mac(KVM_OUI),
            connection_type,
        });
    }
    Ok(interfaces)
}

/// Wait for the VM's ready port until the manifest's ready timeout, recording the host key it
/// offers, and record it running (stage 6). A VM that is not ready in time is left `Starting`,
/// for the scanner to record running once its probe answers; the redeploy still succeeds.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
async fn wait_until_ready(redeploy: &Redeploy<'_>) -> Result<()> {
    let node_name = redeploy.node_name.as_str();
    let progress = redeploy.progress;
    let _ = progress.send_status(
        format!("Waiting for node {node_name} to become ready"),
        StatusKind::Waiting,
    );

    let skip_ready = redeploy.target_node.ready_check.skip.unwrap_or(false);
    if skip_ready {
        let _ = progress.send_status(
            format!("Node {node_name} - Ready check skipped"),
            StatusKind::Done,
        );
    }

    if skip_ready || poll_until_ready(redeploy).await? {
        node_lifecycle::record(
            &redeploy.state.db,
            redeploy.operation,
            &redeploy.node_record_id,
            OperationEvent::Ready,
        )
        .await?;
    } else {
        let _ = progress.send_status(
            format!("Node {node_name} did not become ready within timeout"),
            StatusKind::Waiting,
        );
    }
    Ok(())
}

/// Poll the VM's ready port until it answers or the ready timeout passes. Returns whether it
/// answered, and records the host key it offers.
async fn poll_until_ready(redeploy: &Redeploy<'_>) -> Result<bool> {
    let node_name = redeploy.node_name.as_str();
    let target_node = &redeploy.target_node;
    let ready_port = node_ops::vm_ready_port(target_node.model, target_node.ready_check.port);
    let ready_label = node_ops::ready_port_label(ready_port);
    let ip = redeploy.ipv4.to_string();
    let start = Instant::now();

    while start.elapsed() < redeploy.ready_timeout {
        match node_ops::check_vm_ready(&ip, ready_port).await? {
            VmReadiness::Ready { host_key } => {
                if host_key.is_some() {
                    db::update_node_host_key(
                        &redeploy.state.db,
                        redeploy.node_record_id.clone(),
                        host_key.as_deref(),
                    )
                    .await?;
                }
                let _ = redeploy.progress.send_status(
                    format!("Node {node_name} - Ready ({ready_label} reachable)"),
                    StatusKind::Done,
                );
                return Ok(true);
            }
            VmReadiness::NotReady => {
                let _ = redeploy.progress.send_status(
                    format!("Node {node_name} - Waiting for {ready_label}"),
                    StatusKind::Waiting,
                );
                redeploy
                    .operation
                    .sleep(Duration::from_secs(READINESS_SLEEP))
                    .await?;
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    use shared::data::{BridgeKind, ConnectionTypes, LinkImpairment};

    fn link(node_a: &str, int_a: &str, node_b: &str, int_b: &str) -> DbLink {
        DbLink {
            id: None,
            index: 0,
            kind: BridgeKind::P2p,
            node_a: RecordId::new("node", node_a),
            node_b: RecordId::new("node", node_b),
            int_a: int_a.to_string(),
            int_b: int_b.to_string(),
            lab: RecordId::new("lab", "lab"),
            tap_a: "tapa".to_string(),
            tap_b: "tapb".to_string(),
            impairment_a_to_b: LinkImpairment::default(),
            impairment_b_to_a: LinkImpairment::default(),
        }
    }

    #[test]
    fn each_data_interface_takes_its_link_or_bridge_or_is_disabled() {
        let mut image = data::NodeConfig::ubuntu_linux();
        image.interfaces.reserved_count = 1;
        let first = util::interface_from_idx(&image.model, 2).unwrap();
        let second = util::interface_from_idx(&image.model, 3).unwrap();
        let third = util::interface_from_idx(&image.model, 4).unwrap();
        // The node is on the B end of its link.
        let peer_link = link("peer", "eth1", "vm", &first);
        let other_link = link("peer", &second, "other", &second);
        let bridges = HashMap::from([(second.clone(), "brs0-lab".to_string())]);

        let interfaces = vm_interfaces(
            &image,
            &RecordId::new("node", "vm"),
            "52:54:00:00:00:01",
            2..=4,
            &[&peer_link, &other_link],
            &bridges,
        )
        .unwrap();

        let summary: Vec<(u8, &str, &ConnectionTypes)> = interfaces
            .iter()
            .map(|i| (i.num, i.name.as_str(), &i.connection_type))
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    0,
                    util::dasher(&image.interfaces.management.to_string()).as_str(),
                    &ConnectionTypes::Management
                ),
                (1, "int1", &ConnectionTypes::Reserved),
                (2, "tapb", &ConnectionTypes::P2p),
                (3, "brs0-lab", &ConnectionTypes::PrivateBridge),
                (4, util::dasher(&third).as_str(), &ConnectionTypes::Disabled),
            ]
        );
        assert_eq!(interfaces[0].mac_address, "52:54:00:00:00:01");
        assert_eq!(interfaces[2].mtu, MTU_JUMBO_NET);
    }
}
