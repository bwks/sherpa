use anyhow::{Context, Result, anyhow};
use bollard::secret::ContainerStateStatusEnum;
use libvirt::is_device_volume;
use shared::data::{
    BridgeInfo, DbLab, DbNode, DeviceInfo, InspectResponse, LabInfo, LabState, LinkInfo,
    NodeConfig, NodeKind, NodeState, Observation, Probe, RecordId,
};
use shared::konst::{LAB_FILE_NAME, SHERPA_LABS_PATH, SHERPA_STORAGE_POOL};
use shared::util::load_file;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Instant;
use tracing::Instrument;
use tracing::instrument;
use uuid::Uuid;
use virt::storage_pool::StoragePool;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::node_lifecycle;

/// Inspect a lab and return its current state
///
/// This function queries:
/// - Database for lab metadata and nodes
/// - libvirt for VM disks in the storage pool
/// - Docker for the containers' states, which are reported as the scanner would record them
///   (`reported_state`); the record is left to the scanner
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

    let lab_info = load_lab_info(state, lab_id, &db_lab).await?;

    tracing::debug!("inspect_lab: load lab file took {:?}", t1.elapsed());
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

    let links: Vec<LinkInfo> = db_links
        .into_iter()
        .map(|link| LinkInfo {
            node_a_name: node_name_from_id(&db_nodes, &link.node_a),
            int_a: link.int_a,
            node_b_name: node_name_from_id(&db_nodes, &link.node_b),
            int_b: link.int_b,
            kind: link.kind,
        })
        .collect();

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

    let devices = devices(state, &db_nodes, &node_images, &volumes, lab_id).await?;
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
#[instrument(skip_all, fields(%lab_id), level = "debug")]
async fn devices(
    state: &AppState,
    db_nodes: &[DbNode],
    node_images: &[NodeConfig],
    volumes: &[String],
    lab_id: &str,
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

        let mut device = device_info(node, node_image, volumes, lab_id);
        device.state = reported_state(node, &node_image.kind, lab_id, containers.as_ref());
        if device.state != node.state {
            tracing::debug!(node = %node.name, recorded = %node.state, reported = %device.state, "Reporting the container's state from Docker");
        }
        devices.push(device);
    }
    Ok(devices)
}

/// The lab's details: from its lab file, or from its record when it has none, as for a lab `up`
/// has not written its files for yet, or whose files a destroy removed. Either way it can still
/// be inspected and destroyed.
async fn load_lab_info(state: &AppState, lab_id: &str, db_lab: &DbLab) -> Result<LabInfo> {
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let mut lab_info = match load_file(&format!("{lab_dir}/{LAB_FILE_NAME}")) {
        Ok(lab_file) => LabInfo::from_str(&lab_file).context("Failed to parse lab info file")?,
        Err(error) => {
            tracing::debug!(error = %error, "No lab file; describing the lab from its record");
            let owner = db::find_user_by_id(&state.db, db_lab.user.clone())
                .await
                .context("Failed to look up the lab owner")?
                .map_or_else(|| "unknown".to_string(), |user| user.username);
            lab_info_from_record(db_lab, owner)?
        }
    };
    // The database record is authoritative, and lab files written before it was recorded lack
    // the description and where the lab was started from.
    lab_info.description.clone_from(&db_lab.description);
    lab_info.client_host.clone_from(&db_lab.client_host);
    lab_info.client_folder.clone_from(&db_lab.client_folder);
    Ok(lab_info)
}

/// A lab's details from its record alone, for a lab with no lab file: the networks it reserved,
/// its owner, and what the user said about it.
fn lab_info_from_record(lab: &DbLab, owner: String) -> Result<LabInfo> {
    Ok(LabInfo {
        ssh_usernames: Vec::new(),
        id: lab.lab_id.clone(),
        name: lab.name.clone(),
        user: owner,
        ipv4_network: lab.management_network.parse()?,
        ipv4_gateway: lab.gateway_ipv4.parse()?,
        ipv4_router: lab.router_ipv4.parse()?,
        loopback_network: lab.loopback_network.parse()?,
        ipv6_network: lab
            .management_network_v6
            .as_deref()
            .map(str::parse)
            .transpose()?,
        ipv6_gateway: lab.gateway_ipv6.as_deref().map(str::parse).transpose()?,
        ipv6_router: lab.router_ipv6.as_deref().map(str::parse).transpose()?,
        description: lab.description.clone(),
        client_host: lab.client_host.clone(),
        client_folder: lab.client_folder.clone(),
    })
}

/// A node's details: its state, addresses and resources from the database, and its disks
/// from the storage pool.
fn device_info(node: &DbNode, image: &NodeConfig, volumes: &[String], lab_id: &str) -> DeviceInfo {
    let device_name = format!("{}-{}", node.name, lab_id);
    let mut disks = Vec::new();
    for volume in volumes {
        if is_device_volume(volume, &device_name) {
            disks.push(volume.clone());
        }
    }
    DeviceInfo {
        name: node.name.clone(),
        model: image.model,
        kind: image.kind.clone(),
        state: node.state,
        mgmt_ipv4: node.mgmt_ipv4.clone().unwrap_or_default(),
        mgmt_ipv6: node.mgmt_ipv6.clone(),
        vnc_port: node.vnc_port,
        disks,
        cpu_count: node.cpu_count,
        memory: node.memory,
    }
}

/// The state to report for `node` of `kind` in lab `lab_id`. A container's comes from Docker's
/// `containers` listing, moved from the recorded state as the scanner would record it: a running
/// container that exited is `Stopped`, a gone one `Missing`, and a failed or building node stays
/// as recorded. Other nodes, and containers when Docker did not answer (`None`), are reported as
/// recorded.
fn reported_state(
    node: &DbNode,
    kind: &NodeKind,
    lab_id: &str,
    containers: Option<&HashMap<String, ContainerStateStatusEnum>>,
) -> NodeState {
    let Some(containers) = containers.filter(|_| *kind == NodeKind::Container) else {
        return node.state;
    };
    let status = containers.get(&format!("{}-{lab_id}", node.name)).copied();
    node.state.on_observation(Observation {
        runtime: node_lifecycle::container_runtime(status),
        probe: Probe::NotProbed,
    })
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
    use shared::data::{LabPhase, LabSshKey, NodeModel};

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
            cpu_pinning: None,
            qemu_thread_cpus: None,
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
        let state = reported_state(&node, &NodeKind::Container, "ab12cd34", Some(&containers));
        assert_eq!(state, NodeState::Stopped);
    }

    #[test]
    fn a_container_that_is_gone_is_reported_missing() {
        let node = container_in(NodeState::Running);
        let state = reported_state(
            &node,
            &NodeKind::Container,
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
                &NodeKind::Container,
                "ab12cd34",
                Some(&containers),
            );
            assert_eq!(state, reported, "{recorded} with Docker {status:?}");
        }
    }

    #[test]
    fn a_container_is_reported_as_recorded_when_docker_did_not_answer() {
        let node = container_in(NodeState::Running);
        let state = reported_state(&node, &NodeKind::Container, "ab12cd34", None);
        assert_eq!(state, NodeState::Running);
    }

    #[test]
    fn a_virtual_machine_is_reported_as_recorded() {
        let node = container_in(NodeState::Running);
        let containers = listing(ContainerStateStatusEnum::EXITED);
        let state = reported_state(
            &node,
            &NodeKind::VirtualMachine,
            "ab12cd34",
            Some(&containers),
        );
        assert_eq!(state, NodeState::Running);
    }

    #[test]
    fn device_details_come_from_the_database_and_pool() {
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let volumes = vec![
            "dev01-a1b2c3d4-hdd.qcow2".to_string(),
            "dev02-a1b2c3d4-hdd.qcow2".to_string(),
        ];
        let device = device_info(&db_node("dev01"), &image, &volumes, "a1b2c3d4");
        assert_eq!(device.name, "dev01");
        assert_eq!(device.mgmt_ipv4, "172.31.0.21");
        assert_eq!(device.vnc_port, Some(5903));
        assert_eq!(device.cpu_count, Some(16));
        assert_eq!(device.memory, Some(32768));
        assert_eq!(device.disks, vec!["dev01-a1b2c3d4-hdd.qcow2"]);
    }

    #[test]
    fn a_lab_without_files_is_described_from_its_record() {
        // M6.3: `up` stopped before it wrote the lab's files; `sherpa destroy` inspects first.
        let lab = DbLab {
            id: Some(RecordId::new("lab", "l1")),
            lab_id: "ab12cd34".to_string(),
            name: "core".to_string(),
            user: RecordId::new("user", "alice"),
            loopback_network: "127.127.1.0/24".to_string(),
            management_network: "172.31.1.0/24".to_string(),
            gateway_ipv4: "172.31.1.1".to_string(),
            router_ipv4: "172.31.1.2".to_string(),
            tailscale_container_id: None,
            management_network_v6: Some("fd00:b00b:0:1::/64".to_string()),
            gateway_ipv6: Some("fd00:b00b:0:1::1".to_string()),
            router_ipv6: None,
            loopback_network_v6: None,
            phase: LabPhase::Interrupted,
            client_host: Some("dev01".to_string()),
            client_folder: None,
            description: Some("BGP".to_string()),
            ssh_key: LabSshKey::Lab,
        };

        let info = lab_info_from_record(&lab, "alice".to_string()).unwrap();

        assert_eq!(info.id, "ab12cd34");
        assert_eq!(info.user, "alice");
        assert_eq!(info.ipv4_network.to_string(), "172.31.1.0/24");
        assert_eq!(info.ipv4_router.to_string(), "172.31.1.2");
        assert_eq!(info.loopback_network.to_string(), "127.127.1.0/24");
        assert_eq!(
            info.ipv6_gateway
                .map(|gateway| gateway.to_string())
                .as_deref(),
            Some("fd00:b00b:0:1::1")
        );
        assert!(info.ipv6_router.is_none());
        assert_eq!(info.client_host.as_deref(), Some("dev01"));
        assert_eq!(info.description.as_deref(), Some("BGP"));
    }
}
