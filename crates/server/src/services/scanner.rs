use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use shared::data::{
    DbBridge, DbLab, DbNode, LabPhase, ModelKind, NodeConfig, NodeKind, NodeModel, NodeState,
    Observation, Probe, RecordId, Runtime,
};
use shared::konst::{SHERPA_LABS_PATH, SSH_PORT};
use tokio::time;
use tokio_util::sync::CancellationToken;
use topology::Manifest;
use tracing::instrument;

use crate::daemon::state::AppState;
use crate::daemon::vault_seals::VaultSeals;
use crate::konst::LAB_CERTS_DIR;
use crate::services::lab_definition::stored_definition;
use crate::services::lab_operations;
use crate::services::node_lifecycle::{self, Runtimes, query_runtimes};
use crate::services::node_ops::{self, VmReadiness};
use crate::services::p2p_links::{
    BridgeEnd, ContainerNode, KernelHost, LabLinks, LinkHost, PlannedNode, bridge_ends,
    check_lab_links, follow_carrier, recreated_end, repair_node,
};
use crate::services::ssh_config;
use crate::services::vault::api::{SealStatus, VaultClient};
use crate::services::vault::{self, manages_vault};
use crate::tls::lab_ca;

/// A node with what the scanner needs to know about its image.
struct ScannedNode<'a> {
    node: &'a DbNode,
    kind: NodeKind,
    model: Option<NodeModel>,
    runtime_name: String,
}

/// A running Vault node Sherpa manages, with the seal status read from it this cycle.
struct VaultCheck {
    node_name: String,
    client: VaultClient,
    dir: PathBuf,
    status: SealStatus,
}

/// Run the background scanner service.
///
/// Periodically queries libvirt and Docker for the runtime state of every node, and records
/// what it sees through the node lifecycle (`node_lifecycle::observe`). It only observes a lab
/// no operation holds, and queries the runtimes again under that hold, so it never overwrites a
/// state an operation is writing. Nodes being built and failed nodes are never moved by it.
///
/// A node's readiness is decided by the check stored with it (`DbNode::ready_check`), never by
/// the lab's definition. A running node with a stored port and a check that is not skipped, that
/// is not yet known ready, is probed on that port first, and recorded running only once it
/// answers. So a node whose readiness wait timed out, or that was started outside Sherpa,
/// becomes `Running` once it answers, and one whose port never opens stays `Starting`. A node
/// with no stored port, or a skipped check, is ready once it runs. A running node whose
/// stored port is SSH and has no host key recorded is probed on SSH for the key it offers,
/// skipped or not.
///
/// Under the same hold, in a `Ready` lab, transmit checksum offload goes off again on the
/// management veth of a running container that needs it (`repair_management_checksums`), and
/// the P2p links are checked in the kernel, the ends of running nodes that were made again
/// outside Sherpa being repaired and each link's carrier made to follow its state
/// (`repair_links`). Outside the hold, the seal status of each running Vault node Sherpa manages
/// is read; under the hold, in a `Ready` lab, one found sealed is unsealed with its saved key,
/// and one that cannot be is kept for `inspect` (`unseal_vaults`). The scanner never initialises
/// a Vault. Exits cleanly when the cancellation token is triggered.
#[instrument(skip_all)]
pub async fn run_scanner(state: AppState, cancel: CancellationToken) {
    let interval_secs = state.config.scanner.interval_secs;
    tracing::info!(interval_secs = interval_secs, "Scanner service started");

    let mut interval = time::interval(Duration::from_secs(interval_secs));
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
    let snapshot = query_runtimes(&state.docker, &state.qemu).await;

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
    for node in &scanned {
        let running = snapshot.runtime_of(node.kind, &node.runtime_name) == Some(Runtime::Running);
        let Some(port) = probe_port(node, running) else {
            continue;
        };
        let probe = probe_node(&state.db, node.node, port).await;
        probes.insert(node.node.name.clone(), probe);
    }
    let vault_checks = read_vault_seals(state, lab, &scanned, snapshot).await;

    if let Some(observed) = lab_operations::try_observe(&lab.lab_id) {
        // Under the hold: what an operation wrote before it ended is in the database, and what
        // it did to the runtimes is in this query.
        let runtimes = query_runtimes(&state.docker, &state.qemu).await;
        let nodes = db::list_nodes_by_lab(&state.db, lab_record_id.clone())
            .await
            .context("Failed to list nodes for lab")?;
        let scanned = scanned_nodes(state, lab, &nodes).await?;
        for node in &scanned {
            let Some(runtime) = runtimes.runtime_of(node.kind, &node.runtime_name) else {
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
            repair_management_checksums(
                &KernelHost::new(&state.docker, &state.qemu),
                &scanned,
                &runtimes,
            )
            .await;
            if let Err(error) = repair_links(state, lab, &nodes, &scanned, &runtimes).await {
                tracing::warn!(error = %error, "Failed to check the lab's P2p links");
            }
            unseal_vaults(
                &state.vault_seals,
                &lab.lab_id,
                &scanned,
                &runtimes,
                &vault_checks,
            )
            .await;
        }
    } else {
        tracing::debug!("An operation holds the lab; leaving it for the next cycle");
    }
    Ok(())
}

/// Repair the lab's P2p links and private bridge members, set carrier, and keep link states.
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
    let bridges = db::list_bridges(&state.db, lab_record_id)
        .await
        .map_err(|error| format!("{error:#}"));
    let bridge_ends = match &bridges {
        Ok(bridges) => container_bridge_ends(&lab.lab_id, scanned, bridges),
        Err(error) => {
            tracing::warn!(%error, "Failed to list the lab's bridges");
            vec![]
        }
    };
    if links.is_empty() && bridge_ends.is_empty() {
        return Ok(());
    }
    let kinds = scanned
        .iter()
        .filter_map(|node| Some((node.node.id.clone()?, node.kind)))
        .collect();
    let lab_links = LabLinks::new(&lab.lab_id, links, nodes, kinds);
    let running: HashSet<RecordId> = scanned
        .iter()
        .filter(|node| {
            runtimes.runtime_of(node.kind, &node.runtime_name) == Some(Runtime::Running)
                && node.node.state != NodeState::Building
        })
        .filter_map(|node| node.node.id.clone())
        .collect();

    let host = KernelHost::new(&state.docker, &state.qemu);
    let mut check = check_lab_links(
        &host,
        &lab_links,
        &bridge_ends,
        &running,
        &state.unreadable_ends,
    )
    .await;
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
        // bridges and the stored definition's interface count.
        let has_container = broken.iter().any(|node| node.kind == NodeKind::Container);
        let definition = if has_container {
            stored_definition(&state.db, lab_record_id, &lab.lab_id).await
        } else {
            None
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
            // A VM reads its interfaces only as it boots: a veth made now would never reach
            // it, so a VM-run node's links are made again only when it starts.
            if runs_in_vm(node) {
                tracing::warn!(node = %node.node.name, "A link end of a VM-run node is gone; redeploy the node to make it again");
                continue;
            }
            let planned = PlannedNode {
                lab_id: &lab.lab_id,
                node: node.node,
                node_id,
                kind: &node.kind,
                image: images
                    .iter()
                    .find(|image| image.id.as_ref() == Some(&node.node.image)),
            };
            let Some(end) = recreated_end(
                &planned,
                definition.as_ref(),
                &lab_links,
                bridges.as_deref(),
            ) else {
                continue;
            };
            let outcomes = repair_node(&host, &lab_links, node_id, end).await;
            check.record(&outcomes);
        }
    }
    // Carrier follows a state that changed since the last scan, resume or redeploy recorded
    // it, and is set on the ends made again.
    let previous = state.link_states.of_lab(&lab.lab_id);
    let fresh = check.fresh_ends();
    let states = check.into_states();
    follow_carrier(&host, &lab_links, &previous, &states, &fresh).await;
    state.link_states.replace_lab(&lab.lab_id, states);
    Ok(())
}

/// Whether scanned `node` is a container that runs in its own VM.
fn runs_in_vm(node: &ScannedNode<'_>) -> bool {
    node.kind == NodeKind::Container
        && node
            .model
            .and_then(|model| ModelKind::new(model, Some(NodeKind::Container)).ok())
            .is_some_and(ModelKind::runs_in_vm)
}

/// The private bridge member host ends of the lab's containers.
fn container_bridge_ends(
    lab_id: &str,
    scanned: &[ScannedNode<'_>],
    bridges: &[DbBridge],
) -> Vec<BridgeEnd> {
    let mut ends = vec![];
    for node in scanned {
        let (Some(record), Some(model)) = (&node.node.id, node.model) else {
            continue;
        };
        if node.kind != NodeKind::Container {
            continue;
        }
        let Ok(model_kind) = ModelKind::new(model, Some(node.kind)) else {
            continue;
        };
        let container = ContainerNode {
            lab_id,
            name: &node.node.name,
            index: node.node.index,
            model_kind,
            record,
        };
        match bridge_ends(&container, bridges) {
            Ok(node_ends) => ends.extend(node_ends),
            Err(error) => {
                tracing::warn!(node = %node.node.name, error = %format!("{error:#}"), "Failed to name a container's bridge member ends");
            }
        }
    }
    ends
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
        let Ok(model_kind) = ModelKind::new(model, Some(node.kind)) else {
            continue;
        };
        if node.kind != NodeKind::Container
            || node.node.state == NodeState::Building
            || runtimes.runtime_of(node.kind, &node.runtime_name) != Some(Runtime::Running)
        {
            continue;
        }
        if let Err(error) =
            node_ops::finish_management_checksums(host, &node.runtime_name, model_kind).await
        {
            tracing::warn!(node = %node.node.name, error = %format!("{error:#}"), "Failed to repair checksum offload on the management veth");
        }
    }
}

/// Whether `node` is a Vault node that runs now and that no operation is building.
fn running_vault(node: &ScannedNode<'_>, runtimes: &Runtimes) -> bool {
    node.model == Some(NodeModel::HashicorpVault)
        && node.node.state != NodeState::Building
        && runtimes.runtime_of(node.kind, &node.runtime_name) == Some(Runtime::Running)
}

/// The running Vault nodes Sherpa manages, by the lab's stored definition (`manages_vault`). A
/// Vault node with its own `ztp.config` is never among them.
fn managed_vaults<'a, 'b>(
    scanned: &'b [ScannedNode<'a>],
    runtimes: &Runtimes,
    definition: &Manifest,
) -> Vec<&'b ScannedNode<'a>> {
    scanned
        .iter()
        .filter(|node| running_vault(node, runtimes) && manages_vault(definition, &node.node.name))
        .collect()
}

/// Read the seal status of each running Vault node Sherpa manages, outside the hold. A lab
/// with no running Vault node reads nothing; one with no stored definition, or no lab CA, has
/// none Sherpa can check. A node whose API does not answer is left for the next cycle.
#[instrument(skip_all, fields(lab_id = %lab.lab_id), level = "debug")]
async fn read_vault_seals(
    state: &AppState,
    lab: &DbLab,
    scanned: &[ScannedNode<'_>],
    snapshot: &Runtimes,
) -> Vec<VaultCheck> {
    if !scanned.iter().any(|node| running_vault(node, snapshot)) {
        return Vec::new();
    }
    let Some(lab_record_id) = lab.id.as_ref() else {
        return Vec::new();
    };
    let definition = match db::read_lab_definition(&state.db, lab_record_id).await {
        Ok(Some(definition)) => definition,
        Ok(None) => {
            tracing::debug!("Lab has no stored definition; leaving its Vault nodes alone");
            return Vec::new();
        }
        Err(error) => {
            tracing::debug!(error = %format!("{error:#}"), "Failed to read the lab's stored definition; leaving its Vault nodes alone");
            return Vec::new();
        }
    };
    let managed = managed_vaults(scanned, snapshot, &definition);
    if managed.is_empty() {
        return Vec::new();
    }
    let lab_dir = Path::new(SHERPA_LABS_PATH).join(&lab.lab_id);
    let lab_ca = match lab_ca::load_lab_ca(&lab_dir.join(LAB_CERTS_DIR), &lab.lab_id) {
        Ok(lab_ca) => lab_ca,
        Err(error) => {
            tracing::debug!(error = %format!("{error:#}"), "Failed to load the lab CA; leaving its Vault nodes for the next cycle");
            return Vec::new();
        }
    };
    let mut checks = Vec::new();
    for node in managed {
        let name = &node.node.name;
        let Some(address) = node
            .node
            .mgmt_ipv4
            .as_deref()
            .and_then(|ip| ip.parse::<Ipv4Addr>().ok())
        else {
            tracing::debug!(node = %name, "Vault node has no management address");
            continue;
        };
        match VaultClient::new(address, &lab_ca.pair.cert) {
            Ok(client) => {
                if let Some(check) = vault_check(name, client, vault::vault_dir(&lab.lab_id)).await
                {
                    checks.push(check);
                }
            }
            Err(error) => {
                tracing::debug!(node = %name, error = %format!("{error:#}"), "Failed to build the Vault API client");
            }
        }
    }
    checks
}

/// Read the seal status of Vault node `node_name` through `client`, or `None`, logged, when its
/// API does not answer.
async fn vault_check(node_name: &str, client: VaultClient, dir: PathBuf) -> Option<VaultCheck> {
    match client.seal_status().await {
        Ok(status) => Some(VaultCheck {
            node_name: node_name.to_owned(),
            client,
            dir,
            status,
        }),
        Err(error) => {
            tracing::debug!(node = %node_name, error = %format!("{error:#}"), "Vault seal status not answered");
            None
        }
    }
}

/// Under the hold, in a `Ready` lab: unseal each Vault node `checks` found sealed with its saved
/// key, if it still runs, and keep what was found in `seals` (`vault::record_seal_outcome`). A
/// Vault node that no longer runs has nothing recorded against it.
#[instrument(skip_all, level = "debug", fields(%lab_id))]
async fn unseal_vaults(
    seals: &VaultSeals,
    lab_id: &str,
    scanned: &[ScannedNode<'_>],
    runtimes: &Runtimes,
    checks: &[VaultCheck],
) {
    for node in scanned {
        if node.model == Some(NodeModel::HashicorpVault) && !running_vault(node, runtimes) {
            seals.forget_node(lab_id, &node.node.name);
        }
    }
    for check in checks {
        let still_running = scanned
            .iter()
            .any(|node| node.node.name == check.node_name && running_vault(node, runtimes));
        if !still_running {
            continue;
        }
        let outcome =
            vault::unseal_with_saved_key(&check.client, &check.dir, &check.node_name, check.status)
                .await;
        vault::record_seal_outcome(seals, lab_id, &check.node_name, &outcome);
    }
}

/// The lab's nodes, each with its kind and model.
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
                kind: image.map_or(NodeKind::VirtualMachine, NodeConfig::kind),
                model: image.map(|img| img.model),
                runtime_name: format!("{}-{}", node.name, lab.lab_id),
            }
        })
        .collect())
}

/// The port to probe a node on this cycle, if any: a running node's stored port, when its
/// check is not skipped and it is not yet known ready, or when the port is SSH and it has no host
/// key recorded.
fn probe_port(node: &ScannedNode<'_>, running: bool) -> Option<u16> {
    let port = node.node.ready_check.port?;
    let unready = node_ops::probed_ready_port(&node.node.ready_check).is_some()
        && node.node.state != NodeState::Running;
    let keyless = port == SSH_PORT && node.node.ssh_host_key.is_none();
    (running && (unready || keyless)).then_some(port)
}

/// What the scanner's observation says of a node's readiness: a node whose stored check is
/// probed is ready only when its probe answered this cycle; a node with no port, or a skipped
/// check, has no probe, so running counts as ready.
fn probe_for(node: &ScannedNode<'_>, probed: Option<Probe>) -> Probe {
    if node_ops::probed_ready_port(&node.node.ready_check).is_some() {
        probed.unwrap_or(Probe::NotReady)
    } else {
        Probe::NotProbed
    }
}

/// Probe a running node on `port`, and record the host key it offers, which it does only on
/// SSH. The node's stored check decides what counts as ready, as in `up`: a Windows node's SSH
/// server must show Sherpa's ready marker (`ReadyCheck::sshd_marker`). Returns the probe's
/// result.
#[instrument(skip_all, fields(node = %node.name, port), level = "debug")]
async fn probe_node(db: &db::Database, node: &DbNode, port: u16) -> Probe {
    let Some(ip) = &node.mgmt_ipv4 else {
        tracing::debug!("Node has no management address");
        return Probe::NotReady;
    };
    match node_ops::check_vm_ready(ip, port, node.ready_check.sshd_marker).await {
        Ok(VmReadiness::Ready {
            host_key: Some(host_key),
        }) => match ssh_config::record_node_host_key(db, node, &host_key).await {
            Ok(_) => Probe::Ready,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to record node host key");
                Probe::Ready
            }
        },
        Ok(VmReadiness::Ready { host_key: None }) => Probe::Ready,
        Ok(VmReadiness::NotReady) => {
            tracing::debug!("Ready port not answering yet");
            Probe::NotReady
        }
        Err(e) => {
            tracing::debug!(error = %e, "Probe failed");
            Probe::NotReady
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::secret::ContainerStateStatusEnum;
    use shared::data::{NodeModel, ReadyCheck};
    use tempfile::TempDir;

    use crate::services::node_ops::tests::TestSshd;
    use crate::services::p2p_links::fake::FakeHost;
    use crate::services::vault::lab::tests::{absent_vault, sealed_test_vault};
    use shared::data::VaultSealed;

    /// A virtual machine's stored check on SSH, as `up` stores it for most models.
    const SSH_CHECK: ReadyCheck = ReadyCheck {
        port: Some(SSH_PORT),
        skip: false,
        sshd_marker: false,
    };

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
            ssh_host_key: has_key.then(|| "ssh-ed25519 AAAA".to_string()),
            ready_check: ReadyCheck::default(),
            ssh_username: None,
        }
    }

    /// A node in `state` whose stored check is `ready_check`.
    fn checked(state: NodeState, has_key: bool, ready_check: ReadyCheck) -> DbNode {
        DbNode {
            ready_check,
            ..db_node(state, has_key)
        }
    }

    fn scanned(node: &DbNode, kind: NodeKind) -> ScannedNode<'_> {
        ScannedNode {
            node,
            kind,
            model: None,
            runtime_name: "r1-abc12345".to_string(),
        }
    }

    /// A container node of `model`, whose container is `runtime_name`.
    fn container<'a>(node: &'a DbNode, model: NodeModel, runtime_name: &str) -> ScannedNode<'a> {
        ScannedNode {
            node,
            kind: NodeKind::Container,
            model: Some(model),
            runtime_name: runtime_name.to_string(),
        }
    }

    /// The state one scan records for a running node, whose port answers when `answers`.
    fn scan(node: &ScannedNode<'_>, answers: bool) -> NodeState {
        let probed = probe_port(node, true).map(|_| {
            if answers {
                Probe::Ready
            } else {
                Probe::NotReady
            }
        });
        node.node.state.on_observation(Observation {
            runtime: Runtime::Running,
            probe: probe_for(node, probed),
        })
    }

    #[test]
    fn node_ready_034_the_scanner_does_not_probe_a_node_not_running_or_already_running() {
        let vm = NodeKind::VirtualMachine;
        // Started outside Sherpa, or not ready when its readiness wait ended.
        for state in [NodeState::Stopped, NodeState::Starting, NodeState::Unknown] {
            let node = checked(state, true, SSH_CHECK);
            assert_eq!(
                probe_port(&scanned(&node, vm), true),
                Some(SSH_PORT),
                "{state}"
            );
        }
        let known = checked(NodeState::Running, true, SSH_CHECK);
        assert_eq!(probe_port(&scanned(&known, vm), true), None);
        let node = checked(NodeState::Starting, false, SSH_CHECK);
        assert_eq!(
            probe_port(&scanned(&node, NodeKind::VirtualMachine), false),
            None
        );
        let portless = checked(NodeState::Starting, false, ReadyCheck::default());
        for kind in [
            NodeKind::VirtualMachine,
            NodeKind::Container,
            NodeKind::Unikernel,
        ] {
            assert_eq!(probe_port(&scanned(&portless, kind), true), None);
        }
    }

    #[test]
    fn node_ready_029_the_scanner_records_a_vm_running_only_once_its_port_answers() {
        // Running in libvirt is not ready.
        let node = checked(NodeState::Starting, true, SSH_CHECK);
        let vm = scanned(&node, NodeKind::VirtualMachine);
        assert_eq!(probe_for(&vm, None), Probe::NotReady);
        assert_eq!(probe_for(&vm, Some(Probe::NotReady)), Probe::NotReady);
        assert_eq!(probe_for(&vm, Some(Probe::Ready)), Probe::Ready);
    }

    #[test]
    fn node_ready_030_the_scanner_probes_a_node_on_its_recorded_port_when_not_ssh() {
        let check = ReadyCheck {
            port: Some(8443),
            skip: false,
            sshd_marker: false,
        };
        let starting = checked(NodeState::Starting, false, check.clone());
        let vm = scanned(&starting, NodeKind::VirtualMachine);
        assert_eq!(probe_port(&vm, true), Some(8443));
        assert_eq!(scan(&vm, false), NodeState::Starting);
        assert_eq!(scan(&vm, true), NodeState::Running);
        // It offers no host key on that port, so a running one is not probed for one.
        let running = checked(NodeState::Running, false, check);
        assert_eq!(
            probe_port(&scanned(&running, NodeKind::VirtualMachine), true),
            None
        );
    }

    #[test]
    fn node_ready_031_a_container_whose_port_does_not_answer_stays_starting_across_scans() {
        let node = checked(
            NodeState::Starting,
            false,
            ReadyCheck {
                port: Some(80),
                skip: false,
                sshd_marker: false,
            },
        );
        let web = scanned(&node, NodeKind::Container);
        assert_eq!(probe_port(&web, true), Some(80));
        for _ in 0..3 {
            assert_eq!(scan(&web, false), NodeState::Starting);
        }
    }

    #[test]
    fn node_ready_032_a_node_with_no_recorded_port_is_running_once_it_runs() {
        let node = checked(NodeState::Starting, false, ReadyCheck::default());
        for kind in [
            NodeKind::Container,
            NodeKind::Unikernel,
            NodeKind::VirtualMachine,
        ] {
            let scanned = scanned(&node, kind);
            assert_eq!(probe_for(&scanned, None), Probe::NotProbed);
            assert_eq!(scan(&scanned, false), NodeState::Running);
        }
    }

    #[test]
    fn node_ready_033_a_node_with_a_skipped_check_is_running_as_soon_as_it_runs() {
        let skipped = ReadyCheck {
            port: Some(SSH_PORT),
            skip: true,
            sshd_marker: false,
        };
        let keyed = checked(NodeState::Starting, true, skipped.clone());
        let vm = scanned(&keyed, NodeKind::VirtualMachine);
        assert_eq!(probe_port(&vm, true), None);
        assert_eq!(scan(&vm, false), NodeState::Running);
        // A skipped container port is not probed at all.
        let container = checked(
            NodeState::Starting,
            false,
            ReadyCheck {
                port: Some(80),
                skip: true,
                sshd_marker: false,
            },
        );
        let web = scanned(&container, NodeKind::Container);
        assert_eq!(probe_port(&web, true), None);
        assert_eq!(scan(&web, false), NodeState::Running);
    }

    /// Left for the known hosts spec.
    #[test]
    fn node_host_keys_013_a_running_vm_without_a_key_is_probed_on_ssh_for_one() {
        let keyless = checked(NodeState::Running, false, SSH_CHECK);
        assert_eq!(
            probe_port(&scanned(&keyless, NodeKind::VirtualMachine), true),
            Some(SSH_PORT)
        );
        // Also with a skipped check.
        let skipped = ReadyCheck {
            port: Some(SSH_PORT),
            skip: true,
            sshd_marker: false,
        };
        let keyless = checked(NodeState::Starting, false, skipped);
        assert_eq!(
            probe_port(&scanned(&keyless, NodeKind::VirtualMachine), true),
            Some(SSH_PORT)
        );
    }

    #[test]
    fn container_vm_011_the_scanner_makes_no_veth_again_only_for_a_vm_run_node() {
        let node = db_node(NodeState::Running, true);
        assert!(runs_in_vm(&container(&node, NodeModel::AristaEos, "c-lab")));
        assert!(runs_in_vm(&container(
            &node,
            NodeModel::NokiaSrlinux,
            "s-lab"
        )));
        assert!(!runs_in_vm(&container(&node, NodeModel::CiscoIol, "r-lab")));
        assert!(!runs_in_vm(&container(&node, NodeModel::Generic, "g-lab")));
        assert!(!runs_in_vm(&scanned(&node, NodeKind::VirtualMachine)));
    }

    #[tokio::test]
    async fn veth_checksums_008_each_scan_turns_offload_off_on_a_running_iol_node_only_when_on() {
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
    async fn veth_checksums_009_a_node_whose_management_interface_is_not_found_stops_no_other() {
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

    #[tokio::test]
    #[ignore = "runs /usr/sbin/sshd"]
    async fn node_ready_035_the_scanner_waits_for_a_windows_nodes_ready_marker() {
        let db = db::open_in_memory().await.unwrap();
        let windows_check = ReadyCheck {
            sshd_marker: true,
            ..SSH_CHECK
        };
        let mut node = checked(NodeState::Starting, false, windows_check);
        node.mgmt_ipv4 = Some("127.0.0.1".to_string());

        // The image's SSH server, before cloudbase-init gives it Sherpa's config.
        let image_config = TestSshd::start(false);
        assert_eq!(
            probe_node(&db, &node, image_config.port).await,
            Probe::NotReady
        );
        let sherpa_config = TestSshd::start(true);
        assert_eq!(
            probe_node(&db, &node, sherpa_config.port).await,
            Probe::Ready
        );
        // A node that does not wait for the marker is ready on the image's SSH server.
        node.ready_check.sshd_marker = false;
        assert_eq!(
            probe_node(&db, &node, image_config.port).await,
            Probe::Ready
        );
    }

    /// A lab definition with Vault nodes `vault01` on Sherpa's config and `vault02` on its own.
    fn vault_definition() -> Manifest {
        toml::from_str(
            r#"
name = "vaults"

[[nodes]]
name = "vault01"
model = "hashicorp_vault"

[[nodes]]
name = "vault02"
model = "hashicorp_vault"

[nodes.ztp]
config = "vault.hcl"
"#,
        )
        .unwrap()
    }

    fn vault_node(name: &str, state: NodeState) -> DbNode {
        DbNode {
            name: name.to_string(),
            ..db_node(state, true)
        }
    }

    /// Runtimes with containers `running` running and `stopped` exited.
    fn container_runtimes(running: &[&str], stopped: &[&str]) -> Runtimes {
        let running = running
            .iter()
            .map(|name| ((*name).to_string(), ContainerStateStatusEnum::RUNNING));
        let stopped = stopped
            .iter()
            .map(|name| ((*name).to_string(), ContainerStateStatusEnum::EXITED));
        Runtimes {
            domains: Some(HashMap::new()),
            containers: Some(running.chain(stopped).collect()),
        }
    }

    #[test]
    fn vault_init_018_the_scanner_checks_only_running_vaults_on_sherpas_config() {
        let vault01 = vault_node("vault01", NodeState::Running);
        let own_config = vault_node("vault02", NodeState::Running);
        let undefined = vault_node("vault03", NodeState::Running);
        let building = vault_node("vault01", NodeState::Building);
        let nodes = [
            container(&vault01, NodeModel::HashicorpVault, "vault01-lab"),
            container(&own_config, NodeModel::HashicorpVault, "vault02-lab"),
            container(&undefined, NodeModel::HashicorpVault, "vault03-lab"),
        ];
        let definition = vault_definition();
        let all_running = container_runtimes(&["vault01-lab", "vault02-lab", "vault03-lab"], &[]);
        let managed: Vec<&str> = managed_vaults(&nodes, &all_running, &definition)
            .iter()
            .map(|node| node.node.name.as_str())
            .collect();
        assert_eq!(managed, ["vault01"]);

        let stopped = container_runtimes(&[], &["vault01-lab"]);
        assert!(managed_vaults(&nodes[..1], &stopped, &definition).is_empty());
        let being_built = [container(
            &building,
            NodeModel::HashicorpVault,
            "vault01-lab",
        )];
        assert!(managed_vaults(&being_built, &all_running, &definition).is_empty());
        // Resume picks its nodes the same way.
        assert!(manages_vault(&definition, "vault01"));
        assert!(!manages_vault(&definition, "vault02"));
    }

    #[tokio::test]
    async fn vault_init_025_a_vault_that_stops_has_no_sealed_state_kept() {
        let seals = VaultSeals::default();
        let _ = seals.record("lab", "vault01", Some(VaultSealed::KeyRefused));
        let vault01 = vault_node("vault01", NodeState::Running);
        let nodes = [container(
            &vault01,
            NodeModel::HashicorpVault,
            "vault01-lab",
        )];
        // Still running and not checked this cycle: kept.
        let running = container_runtimes(&["vault01-lab"], &[]);
        unseal_vaults(&seals, "lab", &nodes, &running, &[]).await;
        assert_eq!(seals.of_lab("lab").len(), 1);
        let stopped = container_runtimes(&[], &["vault01-lab"]);
        let dir = TempDir::new().unwrap();
        let checked = VaultCheck {
            node_name: "vault01".to_string(),
            client: absent_vault(),
            dir: dir.path().to_path_buf(),
            status: SealStatus {
                initialized: true,
                sealed: true,
            },
        };
        unseal_vaults(&seals, "lab", &nodes, &stopped, &[checked]).await;
        assert!(seals.of_lab("lab").is_empty());
    }

    #[tokio::test]
    #[ignore = "requires a Docker daemon and the hashicorp/vault:1.21 image"]
    async fn vault_init_016_the_scanner_unseals_a_sealed_vault_sherpa_initialised() {
        let (vault, dir) = sealed_test_vault("sherpa-test-vault-scanner").await;
        let seals = VaultSeals::default();
        let vault01 = vault_node("vault01", NodeState::Running);
        let nodes = [container(
            &vault01,
            NodeModel::HashicorpVault,
            "vault01-lab",
        )];
        let runtimes = container_runtimes(&["vault01-lab"], &[]);

        let check = vault_check("vault01", vault.client(), dir.path().to_path_buf())
            .await
            .unwrap();
        assert!(check.status.sealed);
        unseal_vaults(&seals, "lab", &nodes, &runtimes, &[check]).await;

        let status = vault.client().seal_status().await.unwrap();
        assert!(status.initialized && !status.sealed, "{status:?}");
        assert!(seals.of_lab("lab").is_empty());
        vault.remove().await;
    }
}
