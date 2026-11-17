use anyhow::{Context, Result, anyhow};
use bollard::secret::ContainerStateStatusEnum;
use libvirt::is_device_volume;
use shared::data::{
    BridgeInfo, DbLink, DbNode, DeviceInfo, InspectResponse, LabInfo, LabState, LinkInfo,
    LinkState, NodeConfig, NodeKind, NodeState, Observation, Probe, RecordId,
};
use shared::konst::SHERPA_STORAGE_POOL;
use shared::util::get_ipv4_addr;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Instant;
use tracing::Instrument;
use tracing::instrument;
use uuid::Uuid;
use virt::storage_pool::StoragePool;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::lab_info;
use crate::services::node_lifecycle;
use crate::services::node_ops;

/// Inspect a lab and return its current state
///
/// This function queries:
/// - Database for lab metadata and nodes
/// - libvirt for VM disks in the storage pool
/// - Docker for the containers' states, which are reported as the scanner would record them
///   (`reported_state`); the record is left to the scanner
/// - the server's in-memory link states (`AppState::link_states`) for each P2p link's last
///   state: none before the scanner first checks the lab
///
/// The caller must own the lab or be an admin.
#[instrument(skip(ctx, state), fields(username = %ctx.username, %lab_id))]
pub async fn inspect_lab(
    ctx: &AuthContext,
    lab_id: &str,
    state: &AppState,
) -> Result<InspectResponse> {
    let req_id = Uuid::now_v7();
    let span = tracing::debug_span!("inspect_lab", %req_id, %lab_id);

    inspect_lab_inner(ctx, lab_id, state).instrument(span).await
}

async fn inspect_lab_inner(
    ctx: &AuthContext,
    lab_id: &str,
    state: &AppState,
) -> Result<InspectResponse> {
    let t0 = Instant::now();

    let db_lab = require_lab_access(ctx, &state.db, lab_id).await?;

    let tailscale = super::tailscale::inspect(state, &db_lab).await;

    let lab_record_id = db_lab
        .id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Lab missing record ID"))?;

    tracing::debug!("inspect_lab: user+lab DB lookups took {:?}", t0.elapsed());
    let t1 = Instant::now();

    let owner = lab_info::lab_owner(&state.db, &db_lab).await?;

    tracing::debug!("inspect_lab: owner lookup took {:?}", t1.elapsed());
    let t2 = Instant::now();

    // Run independent async DB operations concurrently
    let (db_nodes, db_links, db_bridges) = tokio::try_join!(
        async {
            db::list_nodes_by_lab(&state.db, lab_record_id.clone())
                .await
                .context("Failed to list nodes for lab")
        },
        async {
            db::list_links_by_lab(&state.db, lab_record_id.clone())
                .await
                .context("Failed to list links for lab")
        },
        async {
            db::list_bridges(&state.db, &lab_record_id)
                .await
                .context("Failed to list bridges for lab")
        },
    )?;

    let lab_info = lab_info::lab_info_from_records(&db_lab, owner, &db_nodes)?;

    tracing::debug!("inspect_lab: concurrent DB queries took {:?}", t2.elapsed());
    let t3 = Instant::now();

    // Batch-fetch node images instead of per-node queries
    let mut image_ids: Vec<RecordId> = db_nodes.iter().map(|n| n.image.clone()).collect();
    image_ids.dedup();

    let node_images = db::list_node_images_by_ids(&state.db, image_ids)
        .await
        .context("Failed to batch fetch node images")?;

    tracing::debug!("inspect_lab: batch image fetch took {:?}", t3.elapsed());
    let t4 = Instant::now();

    // Connect to libvirt
    let qemu_conn = state
        .qemu
        .connect()
        .context("Failed to connect to libvirt")?;

    // Get storage pool
    let pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL).context(format!(
        "Failed to find storage pool '{SHERPA_STORAGE_POOL}'"
    ))?;

    // List volumes once before the loop instead of per-node
    let volumes = pool.list_volumes().context(format!(
        "Failed to list volumes in pool '{SHERPA_STORAGE_POOL}'"
    ))?;

    tracing::debug!(
        "inspect_lab: libvirt connect+pool+volumes took {:?}",
        t4.elapsed()
    );
    tracing::debug!("inspect_lab: total elapsed {:?}", t0.elapsed());

    let links = link_infos(db_links, &db_nodes, &state.link_states.of_lab(lab_id));

    let bridges: Vec<BridgeInfo> = db_bridges
        .into_iter()
        .map(|bridge| {
            let connected_nodes: Vec<String> = bridge
                .members
                .iter()
                .map(|member| node_name_from_id(&db_nodes, &member.node))
                .collect();
            BridgeInfo {
                bridge_name: bridge.bridge_name,
                network_name: bridge.network_name,
                connected_nodes,
            }
        })
        .collect();

    let devices = devices(state, &db_nodes, &node_images, &volumes, &lab_info).await?;
    let node_states: Vec<NodeState> = devices.iter().map(|device| device.state).collect();

    Ok(InspectResponse {
        tailscale,
        lab_info,
        lab_state: LabState::derive(&node_states),
        lab_phase: db_lab.phase,
        devices,
        inactive_devices: Vec::new(), // Keep field for API compatibility
        links,
        bridges,
    })
}

/// Each node's details, with a container's state as Docker reports it (`reported_state`). When
/// Docker does not answer, containers are reported as recorded.
#[instrument(skip_all, fields(lab_id = %lab_info.id), level = "debug")]
async fn devices(
    state: &AppState,
    db_nodes: &[DbNode],
    node_images: &[NodeConfig],
    volumes: &[String],
    lab_info: &LabInfo,
) -> Result<Vec<DeviceInfo>> {
    let has_containers = node_images
        .iter()
        .any(|image| image.kind == NodeKind::Container);
    let containers = if has_containers {
        node_lifecycle::container_states(&state.docker)
            .await
            .inspect_err(|error| {
                tracing::warn!(%error, "Cannot query Docker; reporting the recorded container states");
            })
            .ok()
    } else {
        None
    };

    let mut devices = Vec::new();
    for node in db_nodes {
        let node_image = find_node_image(node_images, &node.image)
            .ok_or_else(|| anyhow!("Node image not found for node '{}'", node.name))?;

        let mut device = device_info(node, node_image, volumes, lab_info);
        device.state = reported_state(node, node_image.kind, &lab_info.id, containers.as_ref());
        if device.state != node.state {
            tracing::debug!(node = %node.name, recorded = %node.state, reported = %device.state, "Reporting the container's state from Docker");
        }
        devices.push(device);
    }
    Ok(devices)
}

/// A node's details: its state, addresses and resources from the database, and its disks
/// from the storage pool. A VM's or unikernel's serial console is on its index in the lab's
/// loopback network, where `up` and `redeploy` put its telnet chardev.
fn device_info(
    node: &DbNode,
    image: &NodeConfig,
    volumes: &[String],
    lab_info: &LabInfo,
) -> DeviceInfo {
    let device_name = format!("{}-{}", node.name, lab_info.id);
    let mut disks = Vec::new();
    for volume in volumes {
        if is_device_volume(volume, &device_name) {
            disks.push(volume.clone());
        }
    }
    DeviceInfo {
        name: node.name.clone(),
        model: image.model,
        kind: image.kind,
        state: node.state,
        mgmt_ipv4: node.mgmt_ipv4.clone().unwrap_or_default(),
        mgmt_ipv6: node.mgmt_ipv6.clone(),
        vnc_port: node.vnc_port,
        disks,
        cpu_count: node.cpu_count,
        memory: node.memory,
        console_address: console_address(node, image.kind, lab_info),
    }
}

/// The address of `node`'s serial console, or `None` for a container, which has none, or for
/// an index past the lab's loopback network.
fn console_address(node: &DbNode, kind: NodeKind, lab_info: &LabInfo) -> Option<Ipv4Addr> {
    if kind == NodeKind::Container {
        return None;
    }
    get_ipv4_addr(&lab_info.loopback_network, u32::from(node.index))
        .inspect_err(|error| tracing::warn!(node = %node.name, %error, "No console address"))
        .ok()
}

/// The state to report for `node` of `kind` in lab `lab_id`. A container's comes from Docker's
/// `containers` listing, moved from the recorded state as the scanner would record it: a running
/// container that exited is `Stopped`, a gone one `Missing`, and a failed or building node stays
/// as recorded. Inspect does not probe, so a running container whose stored check has a port is
/// taken as not having answered: one waiting for its port stays `Starting`, as the scanner left
/// it. Other nodes, and containers when Docker did not answer (`None`), are reported as
/// recorded.
fn reported_state(
    node: &DbNode,
    kind: NodeKind,
    lab_id: &str,
    containers: Option<&HashMap<String, ContainerStateStatusEnum>>,
) -> NodeState {
    let Some(containers) = containers.filter(|_| kind == NodeKind::Container) else {
        return node.state;
    };
    let status = containers.get(&format!("{}-{lab_id}", node.name)).copied();
    node.state.on_observation(Observation {
        runtime: node_lifecycle::container_runtime(status),
        probe: if node_ops::probed_ready_port(&node.ready_check).is_some() {
            Probe::NotReady
        } else {
            Probe::NotProbed
        },
    })
}

/// Each link for display, with the state the server last found or left it in (`link_states`,
/// by link index). A link with no state has not been checked since the server started.
fn link_infos(
    links: Vec<DbLink>,
    nodes: &[DbNode],
    link_states: &HashMap<u16, LinkState>,
) -> Vec<LinkInfo> {
    links
        .into_iter()
        .map(|link| LinkInfo {
            node_a_name: node_name_from_id(nodes, &link.node_a),
            int_a: link.int_a,
            node_b_name: node_name_from_id(nodes, &link.node_b),
            int_b: link.int_b,
            kind: link.kind,
            state: link_states.get(&link.index).cloned(),
            index: Some(link.index),
        })
        .collect()
}

/// Find a node image by `RecordId` in the pre-fetched list.
fn find_node_image<'a>(images: &'a [NodeConfig], id: &RecordId) -> Option<&'a NodeConfig> {
    images.iter().find(|img| img.id.as_ref() == Some(id))
}

/// Resolve a node `RecordId` to its name using the already-fetched node list.
fn node_name_from_id(nodes: &[DbNode], rid: &RecordId) -> String {
    nodes
        .iter()
        .find(|n| n.id.as_ref() == Some(rid))
        .map_or_else(|| format!("{rid:?}"), |n| n.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{BridgeKind, LinkImpairment, NodeModel};
    use shared::data::{ModelKind, ReadyCheck};

    fn db_node(name: &str) -> DbNode {
        DbNode {
            id: None,
            name: name.to_string(),
            image: RecordId::new("node_image", "ubuntu"),
            index: 1,
            lab: RecordId::new("lab", "lab1"),
            mgmt_ipv4: Some("172.31.0.21".to_string()),
            mgmt_ipv6: None,
            mgmt_mac: None,
            state: NodeState::Running,
            cpu_count: Some(16),
            memory: Some(32768),
            vnc_port: Some(5903),
            ssh_host_key: None,
            ready_check: ReadyCheck::default(),
            ssh_username: None,
        }
    }

    fn listing(status: ContainerStateStatusEnum) -> HashMap<String, ContainerStateStatusEnum> {
        HashMap::from([("db01-ab12cd34".to_string(), status)])
    }

    fn container_in(state: NodeState) -> DbNode {
        DbNode {
            state,
            ..db_node("db01")
        }
    }

    #[test]
    fn a_running_container_that_exited_is_reported_stopped() {
        let node = container_in(NodeState::Running);
        let containers = listing(ContainerStateStatusEnum::EXITED);
        let state = reported_state(&node, NodeKind::Container, "ab12cd34", Some(&containers));
        assert_eq!(state, NodeState::Stopped);
    }

    #[test]
    fn a_container_that_is_gone_is_reported_missing() {
        let node = container_in(NodeState::Running);
        let state = reported_state(
            &node,
            NodeKind::Container,
            "ab12cd34",
            Some(&HashMap::new()),
        );
        assert_eq!(state, NodeState::Missing);
    }

    #[test]
    fn a_container_reported_as_docker_has_it_moves_as_the_scanner_would() {
        let cases = [
            (
                NodeState::Running,
                ContainerStateStatusEnum::RUNNING,
                NodeState::Running,
            ),
            (
                NodeState::Starting,
                ContainerStateStatusEnum::RUNNING,
                NodeState::Running,
            ),
            (
                NodeState::Stopped,
                ContainerStateStatusEnum::PAUSED,
                NodeState::Stopped,
            ),
            (
                NodeState::Running,
                ContainerStateStatusEnum::DEAD,
                NodeState::Failed,
            ),
            // A failed node stays failed until it is redeployed, and one being built is left
            // to the operation building it.
            (
                NodeState::Failed,
                ContainerStateStatusEnum::EXITED,
                NodeState::Failed,
            ),
            (
                NodeState::Failed,
                ContainerStateStatusEnum::RUNNING,
                NodeState::Failed,
            ),
            (
                NodeState::Building,
                ContainerStateStatusEnum::EXITED,
                NodeState::Building,
            ),
        ];
        for (recorded, status, reported) in cases {
            let containers = listing(status);
            let state = reported_state(
                &container_in(recorded),
                NodeKind::Container,
                "ab12cd34",
                Some(&containers),
            );
            assert_eq!(state, reported, "{recorded} with Docker {status:?}");
        }
    }

    #[test]
    fn a_running_container_whose_stored_port_has_not_answered_is_reported_starting() {
        // Inspect does not probe: a container waiting for its port stays as the scanner left it.
        let containers = listing(ContainerStateStatusEnum::RUNNING);
        let ported = |state| DbNode {
            ready_check: ReadyCheck {
                port: Some(80),
                skip: false,
            },
            ..container_in(state)
        };
        for (recorded, reported) in [
            (NodeState::Starting, NodeState::Starting),
            (NodeState::Running, NodeState::Running),
        ] {
            let state = reported_state(
                &ported(recorded),
                NodeKind::Container,
                "ab12cd34",
                Some(&containers),
            );
            assert_eq!(state, reported, "{recorded}");
        }
    }

    #[test]
    fn a_container_is_reported_as_recorded_when_docker_did_not_answer() {
        let node = container_in(NodeState::Running);
        let state = reported_state(&node, NodeKind::Container, "ab12cd34", None);
        assert_eq!(state, NodeState::Running);
    }

    #[test]
    fn a_virtual_machine_is_reported_as_recorded() {
        let node = container_in(NodeState::Running);
        let containers = listing(ContainerStateStatusEnum::EXITED);
        let state = reported_state(
            &node,
            NodeKind::VirtualMachine,
            "ab12cd34",
            Some(&containers),
        );
        assert_eq!(state, NodeState::Running);
    }

    fn db_link(index: u16) -> DbLink {
        DbLink {
            id: None,
            index,
            kind: BridgeKind::P2p,
            node_a: RecordId::new("node", "dev01"),
            node_b: RecordId::new("node", "db01"),
            int_a: format!("eth{}", index + 1),
            int_b: format!("eth{}", index + 1),
            lab: RecordId::new("lab", "lab1"),
            tap_a: format!("tpa{index}"),
            tap_b: format!("tpb{index}"),
            impairment_a_to_b: LinkImpairment::default(),
            impairment_b_to_a: LinkImpairment::default(),
        }
    }

    #[test]
    fn each_link_is_shown_with_its_last_state_and_none_before_it_is_checked() {
        let nodes = [
            DbNode {
                id: Some(RecordId::new("node", "dev01")),
                ..db_node("dev01")
            },
            DbNode {
                id: Some(RecordId::new("node", "db01")),
                ..db_node("db01")
            },
        ];
        let peer_down = LinkState::PeerDown {
            peer: "db01".to_string(),
        };
        let states = HashMap::from([(1, peer_down.clone())]);
        let links = link_infos(vec![db_link(0), db_link(1)], &nodes, &states);
        assert_eq!(links[0].state, None);
        assert_eq!(links[1].state, Some(peer_down));
        assert_eq!(links[1].node_a_name, "dev01");
        assert_eq!(links[1].node_b_name, "db01");
        assert_eq!(links[1].int_a, "eth2");
        assert_eq!(links[0].index, Some(0));
        assert_eq!(links[1].index, Some(1));
    }

    fn lab_info() -> LabInfo {
        "id='a1b2c3d4'\nname='core'\nuser='alice'\nipv4_network='172.31.0.0/24'\nipv4_gateway='172.31.0.1'\nipv4_router='172.31.0.2'\nloopback_network='127.127.3.0/24'"
            .parse()
            .unwrap()
    }

    #[test]
    fn a_vm_and_a_unikernel_have_their_console_on_their_loopback_address() {
        for model in [NodeModel::UbuntuLinux, NodeModel::GenericUnikernel] {
            let image = ModelKind::default_for(model).node_config();
            let node = DbNode {
                index: 4,
                ..db_node("dev01")
            };
            let device = device_info(&node, &image, &[], &lab_info());
            assert_eq!(
                device.console_address,
                Some("127.127.3.4".parse().unwrap()),
                "{model}"
            );
        }
    }

    #[test]
    fn a_container_has_no_console_address() {
        let image = ModelKind::default_for(NodeModel::NokiaSrlinux).node_config();
        assert_eq!(image.kind, NodeKind::Container);
        let device = device_info(&db_node("srl01"), &image, &[], &lab_info());
        assert_eq!(device.console_address, None);
    }

    #[test]
    fn device_details_come_from_the_database_and_pool() {
        let image = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        let volumes = vec![
            "dev01-a1b2c3d4-hdd.qcow2".to_string(),
            "dev02-a1b2c3d4-hdd.qcow2".to_string(),
        ];
        let device = device_info(&db_node("dev01"), &image, &volumes, &lab_info());
        assert_eq!(device.name, "dev01");
        assert_eq!(device.mgmt_ipv4, "172.31.0.21");
        assert_eq!(device.vnc_port, Some(5903));
        assert_eq!(device.cpu_count, Some(16));
        assert_eq!(device.memory, Some(32768));
        assert_eq!(device.disks, vec!["dev01-a1b2c3d4-hdd.qcow2"]);
    }
}
