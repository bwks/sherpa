use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use bollard::secret::ContainerSummaryStateEnum;
use shared::data::{DbLab, DbNode, LabState, NodeKind, NodeState, RecordId};
use shared::konst::{SHERPA_LABS_PATH, SSH_PORT};
use tokio_util::sync::CancellationToken;
use tracing::instrument;
use virt::sys::{VIR_DOMAIN_PAUSED, VIR_DOMAIN_RUNNING, VIR_DOMAIN_SHUTOFF};

use crate::daemon::state::AppState;
use crate::services::node_ops::{self, VmReadiness};
use crate::services::ssh_config;

/// Run the background scanner service.
///
/// Periodically queries libvirt and Docker for actual runtime state of all
/// nodes, then reconciles the database to match. Virtual machines that started, as from the
/// web UI, and running ones without a recorded SSH host key, get their host key read and
/// recorded, so the lab's `known_hosts` file follows nodes that make a new key when they
/// boot. Exits cleanly when the cancellation token is triggered.
#[instrument(skip_all)]
pub async fn run_scanner(state: AppState, cancel: CancellationToken) {
    let interval_secs = state.config.scanner.interval_secs;
    tracing::info!(interval_secs = interval_secs, "Scanner service started");

    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    // The first tick completes immediately; skip it so we don't scan at startup
    // before everything is fully initialized.
    interval.tick().await;
    // Started nodes whose host key is read every cycle until their SSH server answers, by
    // runtime name.
    let mut awaiting_host_key = HashSet::new();

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = scan_cycle(&state, &mut awaiting_host_key).await {
                    tracing::error!(error = %e, "Scanner cycle failed");
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("Scanner service shutting down");
                return;
            }
        }
    }
}

/// Execute a single scan cycle: query runtimes, reconcile with DB.
#[instrument(skip_all, level = "debug")]
async fn scan_cycle(state: &AppState, awaiting_host_key: &mut HashSet<String>) -> Result<()> {
    // Query both runtimes concurrently
    let qemu = state.qemu.clone();
    let docker = state.docker.clone();

    let (vm_states, container_states) =
        tokio::join!(query_libvirt_states(qemu), query_docker_states(docker),);

    let vm_states = match vm_states {
        Ok(states) => states,
        Err(e) => {
            tracing::error!(error = %e, "Failed to query libvirt states");
            HashMap::new()
        }
    };

    let container_states = match container_states {
        Ok(states) => states,
        Err(e) => {
            tracing::error!(error = %e, "Failed to query Docker states");
            HashMap::new()
        }
    };

    // Fetch all labs from DB
    let labs = db::list_labs(&state.db)
        .await
        .context("Failed to list labs")?;

    if labs.is_empty() {
        tracing::trace!("No labs found, skipping scan");
        return Ok(());
    }

    for lab in &labs {
        if let Err(e) = scan_lab(state, lab, &vm_states, &container_states, awaiting_host_key).await
        {
            tracing::warn!(
                lab_id = %lab.lab_id,
                error = %e,
                "Failed to scan lab"
            );
        }
    }

    Ok(())
}

/// Scan a single lab: fetch its nodes, determine actual state, update DB if changed.
#[instrument(skip_all, fields(lab_id = %lab.lab_id), level = "debug")]
async fn scan_lab(
    state: &AppState,
    lab: &DbLab,
    vm_states: &HashMap<String, u32>,
    container_states: &HashMap<String, ContainerSummaryStateEnum>,
    awaiting_host_key: &mut HashSet<String>,
) -> Result<()> {
    let lab_record_id = lab.id.as_ref().context("Lab missing record ID")?;

    let nodes = db::list_nodes_by_lab(&state.db, lab_record_id.clone())
        .await
        .context("Failed to list nodes for lab")?;

    if nodes.is_empty() {
        return Ok(());
    }

    // Batch-fetch node images to determine NodeKind
    let mut image_ids: Vec<RecordId> = nodes.iter().map(|n| n.image.clone()).collect();
    image_ids.dedup();

    let node_images = db::list_node_images_by_ids(&state.db, image_ids)
        .await
        .context("Failed to batch fetch node images")?;

    let mut current_node_states = Vec::with_capacity(nodes.len());
    let mut host_key_reads = Vec::new();

    for node in &nodes {
        let node_id = match &node.id {
            Some(id) => id,
            None => {
                tracing::warn!(node_name = %node.name, "Node missing record ID, skipping");
                current_node_states.push(node.state);
                continue;
            }
        };

        // Determine NodeKind from image
        let image = node_images
            .iter()
            .find(|img| img.id.as_ref() == Some(&node.image));
        let kind = image
            .map(|img| img.kind.clone())
            .unwrap_or(NodeKind::VirtualMachine);
        let has_ssh = image.is_some_and(|img| img.model.has_ssh());

        let runtime_name = format!("{}-{}", node.name, lab.lab_id);
        let detected_state = detect_node_state(&runtime_name, &kind, vm_states, container_states);

        if detected_state != node.state {
            tracing::info!(
                node = %node.name,
                lab_id = %lab.lab_id,
                old_state = %node.state,
                new_state = %detected_state,
                "Node state changed"
            );
            if let Err(e) = db::update_node_state(&state.db, node_id.clone(), detected_state).await
            {
                tracing::warn!(
                    node = %node.name,
                    error = %e,
                    "Failed to update node state"
                );
            }
        }

        if detected_state != NodeState::Running {
            awaiting_host_key.remove(&runtime_name);
        } else if node.state != NodeState::Running {
            awaiting_host_key.insert(runtime_name.clone());
        }
        if wants_host_key(
            &kind,
            has_ssh,
            detected_state,
            awaiting_host_key.contains(&runtime_name),
            node.ssh_host_key.is_some(),
        ) {
            host_key_reads.push((node, runtime_name));
        }

        current_node_states.push(detected_state);
    }

    let mut recorded = false;
    for (node, runtime_name) in host_key_reads {
        match read_host_key(state, node).await {
            Ok(true) => {
                awaiting_host_key.remove(&runtime_name);
                recorded = true;
            }
            Ok(false) => {
                tracing::debug!(node = %node.name, "SSH server not answering yet");
            }
            Err(e) => {
                tracing::warn!(node = %node.name, error = %e, "Failed to record node host key");
            }
        }
    }
    if recorded {
        let lab_dir = format!("{SHERPA_LABS_PATH}/{}", lab.lab_id);
        let lab_info = ssh_config::load_lab_info(&lab_dir).await?;
        ssh_config::refresh_lab_ssh_files(
            &state.db,
            &lab_dir,
            lab_record_id,
            &lab_info,
            state.config.server_ipv4,
        )
        .await?;
    }

    // Derive and persist lab state from current node states
    let derived_lab_state = LabState::derive(&current_node_states);
    if derived_lab_state != lab.status {
        tracing::info!(
            lab_id = %lab.lab_id,
            old_state = %lab.status,
            new_state = %derived_lab_state,
            "Lab state changed"
        );
        if let Err(e) =
            db::update_lab_state(&state.db, lab_record_id.clone(), derived_lab_state).await
        {
            tracing::warn!(
                lab_id = %lab.lab_id,
                error = %e,
                "Failed to update lab state"
            );
        }
    }

    Ok(())
}

/// Whether to read a node's SSH host key this cycle: a running virtual machine with SSH that
/// has just started, or is still awaited since it started, or has no key recorded.
fn wants_host_key(
    kind: &NodeKind,
    has_ssh: bool,
    detected_state: NodeState,
    started: bool,
    has_key: bool,
) -> bool {
    *kind == NodeKind::VirtualMachine
        && has_ssh
        && detected_state == NodeState::Running
        && (started || !has_key)
}

/// Read the host key a running node offers on SSH and record it. Returns `false` when its
/// SSH server does not answer yet.
#[instrument(skip_all, fields(node = %node.name), level = "debug")]
async fn read_host_key(state: &AppState, node: &DbNode) -> Result<bool> {
    let Some(ip) = &node.mgmt_ipv4 else {
        tracing::debug!("Node has no management address");
        return Ok(false);
    };
    match node_ops::check_vm_ready(ip, SSH_PORT).await? {
        VmReadiness::Ready {
            host_key: Some(host_key),
        } => {
            ssh_config::record_node_host_key(&state.db, node, &host_key).await?;
            Ok(true)
        }
        VmReadiness::Ready { host_key: None } | VmReadiness::NotReady => Ok(false),
    }
}

/// Determine the actual node state from runtime data.
fn detect_node_state(
    runtime_name: &str,
    kind: &NodeKind,
    vm_states: &HashMap<String, u32>,
    container_states: &HashMap<String, ContainerSummaryStateEnum>,
) -> NodeState {
    match kind {
        NodeKind::VirtualMachine | NodeKind::Unikernel => match vm_states.get(runtime_name) {
            Some(&state_code) if state_code == VIR_DOMAIN_RUNNING => NodeState::Running,
            Some(&state_code) if state_code == VIR_DOMAIN_SHUTOFF => NodeState::Stopped,
            Some(&state_code) if state_code == VIR_DOMAIN_PAUSED => NodeState::Stopped,
            Some(_) => NodeState::Unknown,
            None => NodeState::Stopped,
        },
        NodeKind::Container => match container_states.get(runtime_name) {
            Some(status) => match status {
                ContainerSummaryStateEnum::RUNNING => NodeState::Running,
                ContainerSummaryStateEnum::EXITED | ContainerSummaryStateEnum::DEAD => {
                    NodeState::Stopped
                }
                ContainerSummaryStateEnum::CREATED => NodeState::Created,
                ContainerSummaryStateEnum::RESTARTING => NodeState::Starting,
                ContainerSummaryStateEnum::PAUSED => NodeState::Stopped,
                ContainerSummaryStateEnum::REMOVING => NodeState::Unknown,
                ContainerSummaryStateEnum::EMPTY => NodeState::Unknown,
            },
            None => NodeState::Stopped,
        },
    }
}

/// Query libvirt for all domain states. Must run in spawn_blocking since
/// libvirt calls are blocking FFI.
async fn query_libvirt_states(qemu: Arc<libvirt::Qemu>) -> Result<HashMap<String, u32>> {
    tokio::task::spawn_blocking(move || -> Result<HashMap<String, u32>> {
        let conn = qemu.connect().context("Failed to connect to libvirt")?;
        let domains = conn
            .list_all_domains(0)
            .context("Failed to list libvirt domains")?;

        let mut states = HashMap::with_capacity(domains.len());
        for domain in &domains {
            let name = match domain.get_name() {
                Ok(name) => name,
                Err(e) => {
                    tracing::debug!(error = %e, "Failed to get domain name, skipping");
                    continue;
                }
            };
            let state_code = match domain.get_state() {
                Ok((state, _reason)) => state,
                Err(e) => {
                    tracing::debug!(domain = %name, error = %e, "Failed to get domain state");
                    continue;
                }
            };
            states.insert(name, state_code);
        }
        Ok(states)
    })
    .await
    .context("libvirt query task panicked")?
}

/// Query Docker for all container states.
async fn query_docker_states(
    docker: Arc<bollard::Docker>,
) -> Result<HashMap<String, ContainerSummaryStateEnum>> {
    let containers = container::list_containers(&docker)
        .await
        .context("Failed to list Docker containers")?;

    let mut states = HashMap::with_capacity(containers.len());
    for c in &containers {
        let name = match &c.names {
            Some(names) if !names.is_empty() => {
                // Docker names have a leading '/'
                names[0].trim_start_matches('/').to_string()
            }
            _ => continue,
        };
        if let Some(status) = c.state {
            states.insert(name, status);
        }
    }
    Ok(states)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn started_and_keyless_running_vms_have_their_host_key_read() {
        let vm = NodeKind::VirtualMachine;
        // Just started, or awaited since it started.
        assert!(wants_host_key(&vm, true, NodeState::Running, true, true));
        // Running without a key.
        assert!(wants_host_key(&vm, true, NodeState::Running, false, false));
        // Running with a key, and nothing new.
        assert!(!wants_host_key(&vm, true, NodeState::Running, false, true));
    }

    #[test]
    fn stopped_containers_and_nodes_without_ssh_are_not_read() {
        let vm = NodeKind::VirtualMachine;
        assert!(!wants_host_key(&vm, true, NodeState::Stopped, true, false));
        assert!(!wants_host_key(&vm, false, NodeState::Running, true, false));
        assert!(!wants_host_key(
            &NodeKind::Container,
            true,
            NodeState::Running,
            true,
            false
        ));
        assert!(!wants_host_key(
            &NodeKind::Unikernel,
            true,
            NodeState::Running,
            true,
            false
        ));
    }

    #[test]
    fn test_detect_vm_running() {
        let mut vm_states = HashMap::new();
        vm_states.insert("router1-abc12345".to_string(), VIR_DOMAIN_RUNNING);
        let container_states = HashMap::new();

        let state = detect_node_state(
            "router1-abc12345",
            &NodeKind::VirtualMachine,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Running);
    }

    #[test]
    fn test_detect_vm_shutoff() {
        let mut vm_states = HashMap::new();
        vm_states.insert("router1-abc12345".to_string(), VIR_DOMAIN_SHUTOFF);
        let container_states = HashMap::new();

        let state = detect_node_state(
            "router1-abc12345",
            &NodeKind::VirtualMachine,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Stopped);
    }

    #[test]
    fn test_detect_vm_paused() {
        let mut vm_states = HashMap::new();
        vm_states.insert("router1-abc12345".to_string(), VIR_DOMAIN_PAUSED);
        let container_states = HashMap::new();

        let state = detect_node_state(
            "router1-abc12345",
            &NodeKind::VirtualMachine,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Stopped);
    }

    #[test]
    fn test_detect_vm_not_found() {
        let vm_states = HashMap::new();
        let container_states = HashMap::new();

        let state = detect_node_state(
            "router1-abc12345",
            &NodeKind::VirtualMachine,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Stopped);
    }

    #[test]
    fn test_detect_unikernel_running() {
        let mut vm_states = HashMap::new();
        vm_states.insert("uk1-abc12345".to_string(), VIR_DOMAIN_RUNNING);
        let container_states = HashMap::new();

        let state = detect_node_state(
            "uk1-abc12345",
            &NodeKind::Unikernel,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Running);
    }

    #[test]
    fn test_detect_container_running() {
        let vm_states = HashMap::new();
        let mut container_states = HashMap::new();
        container_states.insert(
            "switch1-abc12345".to_string(),
            ContainerSummaryStateEnum::RUNNING,
        );

        let state = detect_node_state(
            "switch1-abc12345",
            &NodeKind::Container,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Running);
    }

    #[test]
    fn test_detect_container_exited() {
        let vm_states = HashMap::new();
        let mut container_states = HashMap::new();
        container_states.insert(
            "switch1-abc12345".to_string(),
            ContainerSummaryStateEnum::EXITED,
        );

        let state = detect_node_state(
            "switch1-abc12345",
            &NodeKind::Container,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Stopped);
    }

    #[test]
    fn test_detect_container_dead() {
        let vm_states = HashMap::new();
        let mut container_states = HashMap::new();
        container_states.insert(
            "switch1-abc12345".to_string(),
            ContainerSummaryStateEnum::DEAD,
        );

        let state = detect_node_state(
            "switch1-abc12345",
            &NodeKind::Container,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Stopped);
    }

    #[test]
    fn test_detect_container_created() {
        let vm_states = HashMap::new();
        let mut container_states = HashMap::new();
        container_states.insert(
            "switch1-abc12345".to_string(),
            ContainerSummaryStateEnum::CREATED,
        );

        let state = detect_node_state(
            "switch1-abc12345",
            &NodeKind::Container,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Created);
    }

    #[test]
    fn test_detect_container_restarting() {
        let vm_states = HashMap::new();
        let mut container_states = HashMap::new();
        container_states.insert(
            "switch1-abc12345".to_string(),
            ContainerSummaryStateEnum::RESTARTING,
        );

        let state = detect_node_state(
            "switch1-abc12345",
            &NodeKind::Container,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Starting);
    }

    #[test]
    fn test_detect_container_paused() {
        let vm_states = HashMap::new();
        let mut container_states = HashMap::new();
        container_states.insert(
            "switch1-abc12345".to_string(),
            ContainerSummaryStateEnum::PAUSED,
        );

        let state = detect_node_state(
            "switch1-abc12345",
            &NodeKind::Container,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Stopped);
    }

    #[test]
    fn test_detect_container_not_found() {
        let vm_states = HashMap::new();
        let container_states = HashMap::new();

        let state = detect_node_state(
            "switch1-abc12345",
            &NodeKind::Container,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Stopped);
    }

    #[test]
    fn test_detect_container_removing() {
        let vm_states = HashMap::new();
        let mut container_states = HashMap::new();
        container_states.insert(
            "switch1-abc12345".to_string(),
            ContainerSummaryStateEnum::REMOVING,
        );

        let state = detect_node_state(
            "switch1-abc12345",
            &NodeKind::Container,
            &vm_states,
            &container_states,
        );
        assert_eq!(state, NodeState::Unknown);
    }
}
