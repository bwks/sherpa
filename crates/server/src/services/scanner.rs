use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use bollard::secret::ContainerStateStatusEnum;
use shared::data::{
    DbLab, DbNode, LabPhase, NodeKind, NodeModel, NodeState, Observation, Probe, RecordId, Runtime,
};
use shared::konst::{SHERPA_LABS_PATH, SSH_PORT};
use tokio_util::sync::CancellationToken;
use tracing::instrument;

use crate::daemon::state::AppState;
use crate::services::lab_operations;
use crate::services::node_lifecycle;
use crate::services::node_ops::{self, VmReadiness};
use crate::services::p2p_links::{
    KernelHost, LabLinks, LinkHost, PlannedNode, check_lab_links, load_lab_manifest, recreated_end,
    repair_node,
};
use crate::services::ssh_config;

/// What libvirt and Docker reported in one query. A runtime that did not answer is `None`:
/// unknown this time, not empty, as empty would make every one of its nodes missing.
struct Runtimes {
    domains: Option<HashMap<String, u32>>,
    containers: Option<HashMap<String, ContainerStateStatusEnum>>,
}

/// A node with what the scanner needs to know about its image.
struct ScannedNode<'a> {
    node: &'a DbNode,
    kind: NodeKind,
    has_ssh: bool,
    /// The model of its image, or `None` when the image was not found.
    model: Option<NodeModel>,
    runtime_name: String,
}

/// Run the background scanner service.
///
/// Periodically queries libvirt and Docker for the runtime state of every node, and records
/// what it sees through the node lifecycle (`node_lifecycle::observe`). It only observes a lab
/// no operation holds, and queries the runtimes again under that hold, so it never overwrites a
/// state an operation is writing. Nodes being built and failed nodes are never moved by it.
///
/// Virtual machines with SSH that run and are not yet known ready, or have no host key
/// recorded, are probed on SSH first: the probe records the host key they offer, and a node
/// whose probe answered is recorded running. So a node whose readiness wait timed out, or that
/// was started outside Sherpa, becomes `Running` once it answers.
///
/// Under the same hold, in a `Ready` lab, transmit checksum offload goes off again on the
/// management veth of a running container that needs it (`repair_management_checksums`), and
/// the P2p links are checked in the kernel, the ends of running nodes that were made again
/// outside Sherpa being repaired (`repair_links`). Exits cleanly when the cancellation token is
/// triggered.
#[instrument(skip_all)]
pub async fn run_scanner(state: AppState, cancel: CancellationToken) {
    let interval_secs = state.config.scanner.interval_secs;
    tracing::info!(interval_secs = interval_secs, "Scanner service started");

    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    // The first tick completes immediately; skip it so we don't scan at startup
    // before everything is fully initialized.
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = scan_cycle(&state).await {
                    tracing::error!(error = %e, "Scanner cycle failed");
                }
            }
            () = cancel.cancelled() => {
                tracing::info!("Scanner service shutting down");
                return;
            }
        }
    }
}

/// Execute a single scan cycle: query runtimes, then scan every lab.
#[instrument(skip_all, level = "debug")]
async fn scan_cycle(state: &AppState) -> Result<()> {
    let snapshot = query_runtimes(state).await;

    let labs = db::list_labs(&state.db)
        .await
        .context("Failed to list labs")?;

    for lab in &labs {
        if let Err(e) = scan_lab(state, lab, &snapshot).await {
            tracing::warn!(
                lab_id = %lab.lab_id,
                error = %e,
                "Failed to scan lab"
            );
        }
    }

    Ok(())
}

/// Scan one lab. First, outside any hold, probe the nodes `snapshot` shows running that need
/// it. Then, holding the lab, query the runtimes again and record what is seen. A lab an
/// operation holds is skipped this cycle.
#[instrument(skip_all, fields(lab_id = %lab.lab_id), level = "debug")]
async fn scan_lab(state: &AppState, lab: &DbLab, snapshot: &Runtimes) -> Result<()> {
    let lab_record_id = lab.id.as_ref().context("Lab missing record ID")?;

    let nodes = db::list_nodes_by_lab(&state.db, lab_record_id.clone())
        .await
        .context("Failed to list nodes for lab")?;
    let scanned = scanned_nodes(state, lab, &nodes).await?;

    let mut probes = HashMap::new();
    let mut key_recorded = false;
    for node in &scanned {
        let running = snapshot.runtime_of(node) == Some(Runtime::Running);
        if !wants_probe(node, running) {
            continue;
        }
        let (probe, recorded) = probe_node(state, node.node).await;
        key_recorded |= recorded;
        probes.insert(node.node.name.clone(), probe);
    }

    if let Some(observed) = lab_operations::try_observe(&lab.lab_id) {
        // Under the hold: what an operation wrote before it ended is in the database, and what
        // it did to the runtimes is in this query.
        let runtimes = query_runtimes(state).await;
        let nodes = db::list_nodes_by_lab(&state.db, lab_record_id.clone())
            .await
            .context("Failed to list nodes for lab")?;
        let scanned = scanned_nodes(state, lab, &nodes).await?;
        for node in &scanned {
            let Some(runtime) = runtimes.runtime_of(node) else {
                tracing::debug!(node = %node.node.name, "Runtime not queried this cycle; keeping its state");
                continue;
            };
            let observation = Observation {
                runtime,
                probe: probe_for(node, probes.get(&node.node.name).copied()),
            };
            if let Err(error) =
                node_lifecycle::observe(&state.db, &observed, node.node, observation).await
            {
                tracing::warn!(node = %node.node.name, error = %error, "Failed to update node state");
            }
        }
        // Still under the hold: no resume, redeploy or `up` is making the ends this repairs.
        if lab.phase == LabPhase::Ready {
            repair_management_checksums(&KernelHost::new(&state.docker), &scanned, &runtimes).await;
            if let Err(error) = repair_links(state, lab, &nodes, &scanned, &runtimes).await {
                tracing::warn!(error = %error, "Failed to check the lab's P2p links");
            }
        }
    } else {
        tracing::debug!("An operation holds the lab; leaving it for the next cycle");
    }

    if key_recorded {
        refresh_ssh_files(state, lab, lab_record_id).await?;
    }
    Ok(())
}

/// Check the lab's P2p links in the kernel, repair the ends of running nodes that were made
/// again outside Sherpa (see `p2p_links::repair`), and keep each link's state for `inspect`.
///
/// Called under the scanner's hold on the lab, so it never runs beside an operation that makes
/// ends itself. Only nodes the runtime reports running, and that no operation is building, are
/// checked; nothing is started, stopped or unpaused.
#[instrument(skip_all, fields(lab_id = %lab.lab_id), level = "debug")]
async fn repair_links(
    state: &AppState,
    lab: &DbLab,
    nodes: &[DbNode],
    scanned: &[ScannedNode<'_>],
    runtimes: &Runtimes,
) -> Result<()> {
    let lab_record_id = lab.id.as_ref().context("Lab missing record ID")?;
    let links = db::list_links_by_lab(&state.db, lab_record_id.clone())
        .await
        .context("Failed to list the lab's links")?;
    if links.is_empty() {
        return Ok(());
    }
    let lab_links = LabLinks::new(links, nodes);
    let running: HashSet<RecordId> = scanned
        .iter()
        .filter(|node| {
            runtimes.runtime_of(node) == Some(Runtime::Running)
                && node.node.state != NodeState::Building
        })
        .filter_map(|node| node.node.id.clone())
        .collect();

    let host = KernelHost::new(&state.docker);
    let mut check = check_lab_links(&host, &lab_links, &running).await;
    let broken: Vec<&ScannedNode<'_>> = check
        .broken_nodes()
        .into_iter()
        .filter_map(|id| {
            scanned
                .iter()
                .find(|node| node.node.id.as_ref() == Some(&id))
        })
        .collect();
    if !broken.is_empty() {
        // A container's veths are planned as resume plans them: from its image, the lab's
        // bridges and the saved manifest's interface count.
        let has_container = broken.iter().any(|node| node.kind == NodeKind::Container);
        let manifest = has_container
            .then(|| load_lab_manifest(&lab.lab_id))
            .flatten();
        let bridges = if has_container {
            db::list_bridges(&state.db, lab_record_id)
                .await
                .map_err(|error| format!("{error:#}"))
        } else {
            Ok(vec![])
        };
        let images = db::list_node_images_by_ids(
            &state.db,
            broken.iter().map(|node| node.node.image.clone()).collect(),
        )
        .await
        .context("Failed to fetch the node images")?;
        for node in broken {
            let Some(node_id) = &node.node.id else {
                continue;
            };
            let planned = PlannedNode {
                lab_id: &lab.lab_id,
                node: node.node,
                node_id,
                kind: &node.kind,
                image: images
                    .iter()
                    .find(|image| image.id.as_ref() == Some(&node.node.image)),
            };
            let Some(end) =
                recreated_end(&planned, manifest.as_ref(), &lab_links, bridges.as_deref())
            else {
                continue;
            };
            let outcomes = repair_node(&host, &lab_links, node_id, end).await;
            check.record(&outcomes);
        }
    }
    state
        .link_states
        .replace_lab(&lab.lab_id, check.into_states());
    Ok(())
}

/// Turn transmit checksum offload off on the management veth of each running container whose
/// model needs finished checksums, where it is on. Docker makes that veth again, with offload
/// on, when the container starts, and a start outside Sherpa (`docker start`, `docker restart`)
/// is followed by no Sherpa step that turns it off; TCP to the node's management address then
/// fails. Offload is read from the kernel and only changed when on, so nothing is remembered
/// between scans and a node that is right is left alone.
///
/// Called under the scanner's hold on the lab. A node that fails is logged and the others are
/// still checked.
#[instrument(skip_all, level = "debug")]
async fn repair_management_checksums(
    host: &impl LinkHost,
    scanned: &[ScannedNode<'_>],
    runtimes: &Runtimes,
) {
    for node in scanned {
        let Some(model) = node.model else {
            continue;
        };
        if node.kind != NodeKind::Container
            || node.node.state == NodeState::Building
            || runtimes.runtime_of(node) != Some(Runtime::Running)
        {
            continue;
        }
        if let Err(error) =
            node_ops::finish_management_checksums(host, &node.runtime_name, model).await
        {
            tracing::warn!(node = %node.node.name, error = %format!("{error:#}"), "Failed to repair checksum offload on the management veth");
        }
    }
}

/// The lab's nodes, each with its kind and whether its model has SSH.
async fn scanned_nodes<'a>(
    state: &AppState,
    lab: &DbLab,
    nodes: &'a [DbNode],
) -> Result<Vec<ScannedNode<'a>>> {
    if nodes.is_empty() {
        return Ok(Vec::new());
    }
    let mut image_ids: Vec<RecordId> = nodes.iter().map(|n| n.image.clone()).collect();
    image_ids.dedup();
    let node_images = db::list_node_images_by_ids(&state.db, image_ids)
        .await
        .context("Failed to batch fetch node images")?;
    Ok(nodes
        .iter()
        .map(|node| {
            let image = node_images
                .iter()
                .find(|img| img.id.as_ref() == Some(&node.image));
            ScannedNode {
                node,
                kind: image.map_or(NodeKind::VirtualMachine, |img| img.kind.clone()),
                has_ssh: image.is_some_and(|img| img.model.has_ssh()),
                model: image.map(|img| img.model),
                runtime_name: format!("{}-{}", node.name, lab.lab_id),
            }
        })
        .collect())
}

/// Save the lab's SSH files again, with the host keys just recorded.
async fn refresh_ssh_files(state: &AppState, lab: &DbLab, lab_record_id: &RecordId) -> Result<()> {
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
    Ok(())
}

/// Whether to probe a node this cycle: a running virtual machine with SSH that is not yet
/// known ready, or has no host key recorded.
fn wants_probe(node: &ScannedNode<'_>, running: bool) -> bool {
    node.kind == NodeKind::VirtualMachine
        && node.has_ssh
        && running
        && (node.node.state != NodeState::Running || node.node.ssh_host_key.is_none())
}

/// What the scanner's observation says of a node's readiness: virtual machines with SSH are
/// ready only when their probe answered this cycle; other nodes have no probe, so running
/// counts as ready.
fn probe_for(node: &ScannedNode<'_>, probed: Option<Probe>) -> Probe {
    if node.kind == NodeKind::VirtualMachine && node.has_ssh {
        probed.unwrap_or(Probe::NotReady)
    } else {
        Probe::NotProbed
    }
}

/// Probe a running node on SSH, and record the host key it offers. Returns the probe's result
/// and whether a key was recorded.
#[instrument(skip_all, fields(node = %node.name), level = "debug")]
async fn probe_node(state: &AppState, node: &DbNode) -> (Probe, bool) {
    let Some(ip) = &node.mgmt_ipv4 else {
        tracing::debug!("Node has no management address");
        return (Probe::NotReady, false);
    };
    match node_ops::check_vm_ready(ip, SSH_PORT).await {
        Ok(VmReadiness::Ready {
            host_key: Some(host_key),
        }) => match ssh_config::record_node_host_key(&state.db, node, &host_key).await {
            Ok(_) => (Probe::Ready, true),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to record node host key");
                (Probe::Ready, false)
            }
        },
        Ok(VmReadiness::Ready { host_key: None }) => (Probe::Ready, false),
        Ok(VmReadiness::NotReady) => {
            tracing::debug!("SSH server not answering yet");
            (Probe::NotReady, false)
        }
        Err(e) => {
            tracing::debug!(error = %e, "Probe failed");
            (Probe::NotReady, false)
        }
    }
}

/// Query libvirt and Docker at the same time.
async fn query_runtimes(state: &AppState) -> Runtimes {
    let (domains, containers) = tokio::join!(
        query_libvirt_states(state.qemu.clone()),
        node_lifecycle::container_states(&state.docker),
    );
    Runtimes {
        domains: domains
            .inspect_err(|e| tracing::error!(error = %e, "Failed to query libvirt states"))
            .ok(),
        containers: containers
            .inspect_err(|e| tracing::error!(error = %e, "Failed to query Docker states"))
            .ok(),
    }
}

impl Runtimes {
    /// What the runtime reports for `node`, or `None` when its runtime was not queried.
    fn runtime_of(&self, node: &ScannedNode<'_>) -> Option<Runtime> {
        let name = node.runtime_name.as_str();
        Some(match node.kind {
            NodeKind::VirtualMachine | NodeKind::Unikernel => {
                node_lifecycle::libvirt_runtime(self.domains.as_ref()?.get(name).copied())
            }
            NodeKind::Container => {
                node_lifecycle::container_runtime(self.containers.as_ref()?.get(name).copied())
            }
        })
    }
}

/// Query libvirt for all domain states. Must run in `spawn_blocking` since
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeModel;
    use virt::sys::VIR_DOMAIN_RUNNING;

    use crate::services::p2p_links::fake::FakeHost;

    fn db_node(state: NodeState, has_key: bool) -> DbNode {
        DbNode {
            id: None,
            name: "r1".to_string(),
            image: RecordId::new("node_image", "x"),
            index: 1,
            lab: RecordId::new("lab", "x"),
            mgmt_ipv4: Some("172.31.1.10".to_string()),
            mgmt_ipv6: None,
            mgmt_mac: None,
            state,
            cpu_count: None,
            memory: None,
            vnc_port: None,
            ssh_host_key: has_key.then(|| "ssh-ed25519 AAAA".to_string()),
            cpu_pinning: None,
            qemu_thread_cpus: None,
        }
    }

    fn scanned(node: &DbNode, kind: NodeKind, has_ssh: bool) -> ScannedNode<'_> {
        ScannedNode {
            node,
            kind,
            has_ssh,
            model: None,
            runtime_name: "r1-abc12345".to_string(),
        }
    }

    /// A container node of `model`, whose container is `runtime_name`.
    fn container<'a>(node: &'a DbNode, model: NodeModel, runtime_name: &str) -> ScannedNode<'a> {
        ScannedNode {
            node,
            kind: NodeKind::Container,
            has_ssh: true,
            model: Some(model),
            runtime_name: runtime_name.to_string(),
        }
    }

    #[test]
    fn running_vms_not_known_ready_or_without_a_key_are_probed() {
        let vm = NodeKind::VirtualMachine;
        // Started outside Sherpa, or not ready when its readiness wait ended.
        for state in [NodeState::Stopped, NodeState::Starting, NodeState::Unknown] {
            let node = db_node(state, true);
            assert!(
                wants_probe(&scanned(&node, vm.clone(), true), true),
                "{state}"
            );
        }
        let keyless = db_node(NodeState::Running, false);
        assert!(wants_probe(&scanned(&keyless, vm.clone(), true), true));
        let known = db_node(NodeState::Running, true);
        assert!(!wants_probe(&scanned(&known, vm, true), true));
    }

    #[test]
    fn stopped_nodes_containers_and_nodes_without_ssh_are_not_probed() {
        let node = db_node(NodeState::Starting, false);
        assert!(!wants_probe(
            &scanned(&node, NodeKind::VirtualMachine, true),
            false
        ));
        assert!(!wants_probe(
            &scanned(&node, NodeKind::VirtualMachine, false),
            true
        ));
        assert!(!wants_probe(
            &scanned(&node, NodeKind::Container, true),
            true
        ));
        assert!(!wants_probe(
            &scanned(&node, NodeKind::Unikernel, true),
            true
        ));
    }

    #[test]
    fn a_vm_with_ssh_is_ready_only_when_its_probe_answered() {
        // M1.3: running in libvirt is not ready.
        let node = db_node(NodeState::Starting, true);
        let vm = scanned(&node, NodeKind::VirtualMachine, true);
        assert_eq!(probe_for(&vm, None), Probe::NotReady);
        assert_eq!(probe_for(&vm, Some(Probe::NotReady)), Probe::NotReady);
        assert_eq!(probe_for(&vm, Some(Probe::Ready)), Probe::Ready);
        for (kind, has_ssh) in [
            (NodeKind::Container, true),
            (NodeKind::Unikernel, false),
            (NodeKind::VirtualMachine, false),
        ] {
            assert_eq!(
                probe_for(&scanned(&node, kind, has_ssh), None),
                Probe::NotProbed
            );
        }
    }

    #[test]
    fn a_runtime_that_could_not_be_queried_leaves_its_nodes_undecided() {
        let node = db_node(NodeState::Running, true);
        let containers =
            HashMap::from([("r1-abc12345".to_string(), ContainerStateStatusEnum::RUNNING)]);
        let runtimes = Runtimes {
            domains: None,
            containers: Some(containers),
        };
        // libvirt did not answer: its nodes are undecided, not missing.
        assert_eq!(
            runtimes.runtime_of(&scanned(&node, NodeKind::VirtualMachine, true)),
            None
        );
        assert_eq!(
            runtimes.runtime_of(&scanned(&node, NodeKind::Unikernel, false)),
            None
        );
        // Docker answered, so its nodes are still decided.
        assert_eq!(
            runtimes.runtime_of(&scanned(&node, NodeKind::Container, false)),
            Some(Runtime::Running)
        );
    }

    #[test]
    fn a_node_with_no_domain_or_container_is_absent_not_stopped() {
        // M4.2: missing used to be read as stopped.
        let node = db_node(NodeState::Running, true);
        let runtimes = Runtimes {
            domains: Some(HashMap::from([(
                "other-abc12345".to_string(),
                VIR_DOMAIN_RUNNING,
            )])),
            containers: Some(HashMap::new()),
        };
        assert_eq!(
            runtimes.runtime_of(&scanned(&node, NodeKind::VirtualMachine, true)),
            Some(Runtime::Absent)
        );
        assert_eq!(
            runtimes.runtime_of(&scanned(&node, NodeKind::Container, false)),
            Some(Runtime::Absent)
        );
    }

    #[test]
    fn a_dead_container_is_faulted() {
        // M4.1: a dead container used to look stopped, so the UI offered Start.
        let node = db_node(NodeState::Running, true);
        let runtimes = Runtimes {
            domains: Some(HashMap::new()),
            containers: Some(HashMap::from([(
                "r1-abc12345".to_string(),
                ContainerStateStatusEnum::DEAD,
            )])),
        };
        assert_eq!(
            runtimes.runtime_of(&scanned(&node, NodeKind::Container, false)),
            Some(Runtime::Faulted("dead"))
        );
    }

    #[tokio::test]
    async fn the_management_veth_of_a_running_iol_node_gets_offload_off_only_when_it_is_on() {
        let running = db_node(NodeState::Running, true);
        let building = db_node(NodeState::Building, true);
        let nodes = [
            // Restarted outside Sherpa: Docker made its management veth again, offload on.
            container(&running, NodeModel::CiscoIol, "r1-lab"),
            // Already off: left alone.
            container(&running, NodeModel::CiscoIoll2, "sw1-lab"),
            // Not IOL: never touched, though its offload is on.
            container(&running, NodeModel::NokiaSrlinux, "srl-lab"),
            // Stopped, and being built: not checked.
            container(&running, NodeModel::CiscoIol, "r2-lab"),
            container(&building, NodeModel::CiscoIol, "r3-lab"),
        ];
        let runtimes = Runtimes {
            domains: Some(HashMap::new()),
            containers: Some(HashMap::from([
                ("r1-lab".to_string(), ContainerStateStatusEnum::RUNNING),
                ("sw1-lab".to_string(), ContainerStateStatusEnum::RUNNING),
                ("srl-lab".to_string(), ContainerStateStatusEnum::RUNNING),
                ("r2-lab".to_string(), ContainerStateStatusEnum::EXITED),
                ("r3-lab".to_string(), ContainerStateStatusEnum::RUNNING),
            ])),
        };
        let host = FakeHost {
            management_veths: HashMap::from([
                ("r1-lab".to_string(), 11),
                ("sw1-lab".to_string(), 12),
                ("srl-lab".to_string(), 13),
                ("r2-lab".to_string(), 14),
                ("r3-lab".to_string(), 15),
            ]),
            ..FakeHost::default()
        };
        host.offload_on.borrow_mut().extend([11, 13, 14, 15]);

        repair_management_checksums(&host, &nodes, &runtimes).await;

        assert!(!host.offload_on.borrow().contains(&11), "IOL, offload on");
        assert!(host.offload_on.borrow().contains(&13), "not IOL");
        assert!(host.offload_on.borrow().contains(&14), "stopped");
        assert!(host.offload_on.borrow().contains(&15), "being built");
        assert_eq!(host.writes.get(), 1, "only r1's veth is changed");

        // A second scan finds nothing to change.
        repair_management_checksums(&host, &nodes, &runtimes).await;
        assert_eq!(host.writes.get(), 1);
    }

    #[tokio::test]
    async fn a_node_whose_management_veth_cannot_be_found_does_not_stop_the_others() {
        let running = db_node(NodeState::Running, true);
        let nodes = [
            container(&running, NodeModel::CiscoIol, "gone-lab"),
            container(&running, NodeModel::CiscoIol, "r1-lab"),
        ];
        let runtimes = Runtimes {
            domains: Some(HashMap::new()),
            containers: Some(HashMap::from([
                ("gone-lab".to_string(), ContainerStateStatusEnum::RUNNING),
                ("r1-lab".to_string(), ContainerStateStatusEnum::RUNNING),
            ])),
        };
        let host = FakeHost {
            management_veths: HashMap::from([("r1-lab".to_string(), 11)]),
            ..FakeHost::default()
        };
        host.offload_on.borrow_mut().insert(11);

        repair_management_checksums(&host, &nodes, &runtimes).await;

        assert!(!host.offload_on.borrow().contains(&11));
    }
}
