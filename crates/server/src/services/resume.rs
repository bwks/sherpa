use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use opentelemetry::KeyValue;
use shared::data::{
    self, DbBridge, DbLab, DbLink, Halt, LabNodeActionResponse, NodeActionResult, NodeKind,
    OperationEvent, RecordId, Runtime, StatusKind, TailnetAuthKey, TailnetStatus,
};
use shared::konst::{READINESS_SLEEP, READINESS_TIMEOUT, SHERPA_LABS_PATH, SSH_PORT};
use shared::util::load_saved_manifest;
use shared::validators::effective_data_interface_count;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use topology::Manifest;

use tracing::instrument;

use super::admission::{self, Admission};
use super::lab_lifecycle;
use super::lab_nodes::load_target_nodes;
use super::lab_operations::{self, LabOperation};
use super::node_lifecycle;
use super::node_ops::{self, VmReadiness};
use super::p2p_links::{
    ContainerNode, KernelHost, LabLinks, RecreatedEnd, container_veths, data_interface_indices,
    node_bridge_interfaces, node_links, reattach_node_links, report_links,
};
use super::progress::ProgressSender;
use super::ssh_config;
use crate::auth::context::AuthContext;
use crate::auth::policy::{require_lab_access, require_lab_starter};
use crate::daemon::state::AppState;

/// How a node started by resume shows that it is ready, as `up` checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReadinessCheck {
    /// A virtual machine accepting TCP connections on this port.
    Port(u16),
    Unikernel {
        ready_port: Option<u16>,
    },
}

/// What resume does to a node's domain or container, by what its runtime reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartStep {
    /// Boot it: libvirt `create`, Docker `start`.
    Boot,
    /// Carry on where it was paused: libvirt `resume`, Docker `unpause`.
    Resume,
    /// It is already running.
    Nothing,
}

/// What starting one node did.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StartOutcome {
    /// Booted from off: new taps, so its links are re-attached and its readiness awaited.
    ColdBooted,
    /// Unpaused: the guest carries on, ready as it was.
    Resumed,
    /// Already running: left as it is.
    AlreadyRunning,
    /// Not started, and why.
    Failed(String),
}

/// A node that resume cold-booted and waits for.
#[derive(Debug, Clone)]
struct ReadinessTarget {
    name: String,
    device_name: String,
    mgmt_ipv4: Option<String>,
    check: ReadinessCheck,
}

/// Start/poweron all (or a specific) node(s) for a lab.
///
/// VMs: if shutoff, calls `domain.create()` (cold boot). If paused, calls `domain.resume()`.
/// Containers: calls `docker start`, or `docker unpause` if paused. A crashed or dead node, one
/// between states, one with no domain or container, and one another operation is building are
/// refused.
/// A cold-booted node is recorded `Starting`, then `Running` once it is ready; a resumed one
/// `Running`; one already running is left as it is.
///
/// Cold-booted VMs and unikernels are then polled for readiness like `up` does, using the
/// lab's saved manifest for `ready_timeout`, `ready_check.skip` and `ready_check.port`. A node that
/// does not become ready within the timeout is reported in its result message; it is not an
/// error, as with `up`.
///
/// A whole-lab resume of a lab with a Tailscale gateway enrolls the gateway again with
/// `tailscale_auth_key`, as `down` logged it out. The key is used, never stored.
///
/// The caller must own the lab or be an admin, and may not be the built-in `admin` account.
#[instrument(skip(ctx, tailscale_auth_key, state, progress), fields(username = %ctx.username, %lab_id))]
pub async fn start_lab_nodes(
    ctx: &AuthContext,
    lab_id: &str,
    node_name: Option<&str>,
    tailscale_auth_key: Option<&TailnetAuthKey>,
    state: &AppState,
    progress: Option<&ProgressSender>,
) -> Result<LabNodeActionResponse> {
    let start = Instant::now();

    require_lab_starter(ctx)?;
    require_lab_access(ctx, &state.db, lab_id).await?;
    // Destroying the lab stops this operation; see `lab_operations`.
    let operation = lab_operations::begin(lab_id).await;
    // Check again under the lease: a destroy that finished while this waited has removed the lab,
    // and only a lab `up` has built, and no destroy has started on, has nodes to start.
    let db_lab = require_lab_access(ctx, &state.db, lab_id).await?;
    lab_lifecycle::require_ready(&db_lab)?;

    let lab_record_id = db_lab
        .id
        .clone()
        .ok_or_else(|| anyhow!("Lab missing record ID"))?;

    let (target_nodes, node_images) =
        load_target_nodes(&state.db, lab_record_id.clone(), lab_id, node_name).await?;

    // Stopped VMs and unikernels must fit in the server's memory before any node starts.
    let _admission = admit_stopped_nodes(state, lab_id, &target_nodes).await?;

    let tailscale =
        resume_tailscale(state, &db_lab, node_name, tailscale_auth_key, progress).await?;

    // Build futures for all node operations concurrently
    let mut futures = Vec::new();
    let mut immediate_results = Vec::new();

    for node in &target_nodes {
        let device_name = format!("{}-{}", node.name, lab_id);
        let kind = match startable_kind(node, &node_images) {
            Ok(kind) => kind,
            Err(refused) => {
                immediate_results.push(refused);
                continue;
            }
        };

        let node_name = node.name.clone();
        let node_id = node.id.clone();
        let state = state.clone();

        futures.push(async move {
            let outcome = match kind {
                NodeKind::VirtualMachine | NodeKind::Unikernel => {
                    start_vm(&device_name, &state).await
                }
                NodeKind::Container => start_container_node(&device_name, &state).await,
            };

            (outcome, node_name, node_id, kind)
        });
    }

    // Run all operations concurrently
    let concurrent_results = join_all(futures).await;

    let mut results = immediate_results;
    let cold_booted = record_outcomes(state, &operation, concurrent_results, &mut results).await;
    let manifest = load_lab_manifest(lab_id);
    let lab = ResumedLab {
        lab_id,
        lab_record_id: &lab_record_id,
        nodes: &target_nodes,
        manifest: manifest.as_ref(),
        state,
        progress,
        operation: &operation,
    };

    // A cold boot makes a node's link ends again, so its links are re-attached before its
    // readiness is awaited, as `up` does.
    reattach_cold_booted(&lab, &node_images, &cold_booted, &mut results).await;
    let ready = wait_for_cold_booted(&lab, &cold_booted, &mut results).await?;
    // A node not ready in time stays `Starting`; the scanner records it running once it answers.
    for node in target_nodes
        .iter()
        .filter(|node| ready.contains(&node.name))
    {
        if let Some(id) = &node.id
            && let Err(error) = record_all(state, &operation, id, &[OperationEvent::Ready]).await
        {
            tracing::warn!(node = %node.name, error = %error, "Failed to record the node ready");
        }
    }

    state.metrics.operation_duration.record(
        start.elapsed().as_secs_f64(),
        &[KeyValue::new("operation.type", "resume")],
    );

    Ok(LabNodeActionResponse { results, tailscale })
}

/// Admit the target nodes that are stopped: VMs, unikernels and containers with a memory limit.
/// Their memory must fit beside every running guest's and the nodes other operations are
/// starting.
#[instrument(skip_all, fields(%lab_id), level = "debug")]
async fn admit_stopped_nodes(
    state: &AppState,
    lab_id: &str,
    target_nodes: &[data::DbNode],
) -> Result<Admission> {
    let node_names: Vec<&str> = target_nodes.iter().map(|node| node.name.as_str()).collect();
    let conn = state
        .qemu
        .connect()
        .context("Failed to connect to libvirt")?;
    admission::admit_stopped_nodes(
        &conn,
        &state.docker,
        state.config.host_memory,
        lab_id,
        &node_names,
    )
    .await
}

/// Enroll the lab's Tailscale gateway again on a whole-lab resume with a key, as `down` logged
/// it out; otherwise report its current status.
#[instrument(skip_all, fields(lab_id = %db_lab.lab_id), level = "debug")]
async fn resume_tailscale(
    state: &AppState,
    db_lab: &DbLab,
    node_name: Option<&str>,
    tailscale_auth_key: Option<&TailnetAuthKey>,
    progress: Option<&ProgressSender>,
) -> Result<Option<TailnetStatus>> {
    let credential = super::tailscale::resume_credential(
        db_lab.tailscale_container_id.is_some(),
        node_name.is_none(),
        tailscale_auth_key,
        state.config.tls.enabled,
    )?;
    match credential {
        Some(key) => {
            if let Some(progress) = progress {
                let _ = progress.send_status(
                    "Connecting lab to Tailscale".to_string(),
                    StatusKind::Progress,
                );
            }
            super::tailscale::resume(state, db_lab, key).await
        }
        None => Ok(super::tailscale::inspect(state, db_lab).await),
    }
}

/// The lab being resumed, for waiting on its cold-booted nodes.
#[derive(Clone, Copy)]
struct ResumedLab<'a> {
    lab_id: &'a str,
    lab_record_id: &'a RecordId,
    nodes: &'a [data::DbNode],
    /// The manifest `up` saved, if it can be read.
    manifest: Option<&'a Manifest>,
    state: &'a AppState,
    progress: Option<&'a ProgressSender>,
    operation: &'a LabOperation,
}

/// Wait for the cold-booted VMs and unikernels to become ready, as `up` does, with the saved
/// manifest's readiness settings. Each one's result says whether it became ready, and the host
/// keys it offers are recorded again. Returns the cold-booted nodes that are ready: those that
/// answered, and those with no check, as containers are ready once running.
#[instrument(skip_all, fields(lab_id = %lab.lab_id), level = "debug")]
async fn wait_for_cold_booted(
    lab: &ResumedLab<'_>,
    cold_booted: &[(String, NodeKind)],
    results: &mut [NodeActionResult],
) -> Result<HashSet<String>> {
    let ResumedLab {
        lab_id,
        lab_record_id,
        nodes,
        manifest,
        state,
        progress,
        operation,
    } = *lab;
    let targets = readiness_targets(lab_id, nodes, cold_booted, manifest);
    let mut ready_nodes: HashSet<String> = cold_booted
        .iter()
        .map(|(name, _)| name.clone())
        .filter(|name| !targets.iter().any(|target| &target.name == name))
        .collect();
    if targets.is_empty() {
        return Ok(ready_nodes);
    }
    let timeout = ready_timeout(manifest);
    let ready = wait_for_readiness(lab_id, &targets, timeout, state, progress, operation).await?;
    for result in results.iter_mut() {
        if targets.iter().any(|target| target.name == result.name) {
            result.message = readiness_message(ready.contains_key(&result.name), timeout);
        }
    }
    // A node may make a new host key when it boots, so record the keys again.
    if let Err(error) = save_host_keys(lab_id, lab_record_id, &ready, nodes, state).await {
        tracing::warn!(lab_id = %lab_id, error = ?error, "Failed to save resumed nodes' host keys");
    }
    ready_nodes.extend(ready.into_keys());
    Ok(ready_nodes)
}

/// Load the manifest saved by `up`. Labs without a readable manifest use the defaults.
#[instrument(level = "debug")]
fn load_lab_manifest(lab_id: &str) -> Option<Manifest> {
    let lab_dir = PathBuf::from(format!("{SHERPA_LABS_PATH}/{lab_id}"));
    let value = match load_saved_manifest(&lab_dir) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(lab_id = %lab_id, error = ?error, "No saved manifest; using default readiness settings");
            return None;
        }
    };
    match serde_json::from_value(value) {
        Ok(manifest) => Some(manifest),
        Err(error) => {
            tracing::warn!(lab_id = %lab_id, error = ?error, "Saved manifest unreadable; using default readiness settings");
            None
        }
    }
}

fn ready_timeout(manifest: Option<&Manifest>) -> Duration {
    let seconds = match manifest {
        Some(manifest) => manifest.ready_timeout.unwrap_or(READINESS_TIMEOUT),
        None => READINESS_TIMEOUT,
    };
    Duration::from_secs(seconds)
}

/// Decide how a started node proves readiness. Paused nodes resume an already booted
/// guest, and containers are ready once running, so only cold-booted guests are checked.
fn readiness_check(
    kind: &NodeKind,
    cold_booted: bool,
    manifest_node: Option<&topology::Node>,
) -> Option<ReadinessCheck> {
    let (skip, ready_port) = match manifest_node {
        Some(node) => (
            node.ready_check.skip.unwrap_or(false),
            node.ready_check.port,
        ),
        None => (false, None),
    };
    if !cold_booted || skip {
        return None;
    }
    match kind {
        // Without a saved manifest the model is unknown, so fall back to SSH as before.
        NodeKind::VirtualMachine => Some(ReadinessCheck::Port(match manifest_node {
            Some(node) => node_ops::vm_ready_port(node.model, ready_port),
            None => SSH_PORT,
        })),
        NodeKind::Unikernel => Some(ReadinessCheck::Unikernel { ready_port }),
        NodeKind::Container => None,
    }
}

fn readiness_targets(
    lab_id: &str,
    nodes: &[data::DbNode],
    cold_booted: &[(String, NodeKind)],
    manifest: Option<&Manifest>,
) -> Vec<ReadinessTarget> {
    let mut targets = vec![];
    for (name, kind) in cold_booted {
        let manifest_node = match manifest {
            Some(manifest) => manifest.nodes.iter().find(|node| &node.name == name),
            None => None,
        };
        let Some(check) = readiness_check(kind, true, manifest_node) else {
            tracing::debug!(lab_id = %lab_id, node_name = %name, "No readiness check for node");
            continue;
        };
        let mgmt_ipv4 = match nodes.iter().find(|node| &node.name == name) {
            Some(node) => node.mgmt_ipv4.clone(),
            None => None,
        };
        targets.push(ReadinessTarget {
            name: name.clone(),
            device_name: format!("{name}-{lab_id}"),
            mgmt_ipv4,
            check,
        });
    }
    targets
}

fn readiness_message(ready: bool, timeout: Duration) -> String {
    if ready {
        "Started and ready".to_string()
    } else {
        format!(
            "Started, but not ready within {} seconds",
            timeout.as_secs()
        )
    }
}

fn report(progress: Option<&ProgressSender>, message: String, kind: StatusKind) {
    if let Some(progress) = progress
        && let Err(error) = progress.send_status(message, kind)
    {
        tracing::debug!(error = ?error, "Resume progress receiver closed");
    }
}

/// Probe one node once. Probe errors count as not ready yet.
async fn probe(target: &ReadinessTarget, state: &AppState) -> VmReadiness {
    let outcome = match (&target.check, target.mgmt_ipv4.as_deref()) {
        (ReadinessCheck::Port(port), Some(ip)) => node_ops::check_vm_ready(ip, *port).await,
        (ReadinessCheck::Port(_), None) => Ok(VmReadiness::NotReady),
        (ReadinessCheck::Unikernel { ready_port }, ip) => match state.qemu.connect() {
            Ok(conn) => {
                match node_ops::check_unikernel_ready(&conn, &target.device_name, *ready_port, ip) {
                    Ok(true) => Ok(VmReadiness::Ready { host_key: None }),
                    Ok(false) => Ok(VmReadiness::NotReady),
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        },
    };
    match outcome {
        Ok(readiness) => readiness,
        Err(error) => {
            tracing::debug!(node_name = %target.name, error = ?error, "Readiness probe failed");
            VmReadiness::NotReady
        }
    }
}

/// Record the host keys that resumed nodes offered, and save the lab's SSH files with them.
/// Nodes ready without an SSH handshake keep their recorded keys.
#[instrument(skip(lab_record_id, ready, nodes, state), level = "debug")]
async fn save_host_keys(
    lab_id: &str,
    lab_record_id: &RecordId,
    ready: &HashMap<String, Option<String>>,
    nodes: &[data::DbNode],
    state: &AppState,
) -> Result<()> {
    let mut recorded = false;
    for node in nodes {
        let Some(Some(host_key)) = ready.get(&node.name) else {
            tracing::trace!(node_name = %node.name, "No host key offered");
            continue;
        };
        ssh_config::record_node_host_key(&state.db, node, host_key).await?;
        recorded = true;
    }
    if !recorded {
        tracing::debug!("No host keys to record");
        return Ok(());
    }
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
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

/// Fails if the lab is destroyed meanwhile.
#[instrument(skip(targets, state, progress, operation), fields(%lab_id, nodes = targets.len()), level = "debug")]
async fn wait_for_readiness(
    lab_id: &str,
    targets: &[ReadinessTarget],
    timeout: Duration,
    state: &AppState,
    progress: Option<&ProgressSender>,
    operation: &LabOperation,
) -> Result<HashMap<String, Option<String>>> {
    report(
        progress,
        format!(
            "Waiting for {} nodes to become ready (up to {} seconds)",
            targets.len(),
            timeout.as_secs()
        ),
        StatusKind::Waiting,
    );
    let start = Instant::now();
    let mut ready = HashMap::new();
    loop {
        for target in targets {
            if ready.contains_key(&target.name) {
                continue;
            }
            let VmReadiness::Ready { host_key } = probe(target, state).await else {
                continue;
            };
            tracing::info!(lab_id = %lab_id, node_name = %target.name, "Resumed node ready");
            report(
                progress,
                format!("Node {} - Ready", target.name),
                StatusKind::Done,
            );
            ready.insert(target.name.clone(), host_key);
        }
        if ready.len() == targets.len() || start.elapsed() >= timeout {
            break;
        }
        operation
            .sleep(Duration::from_secs(READINESS_SLEEP))
            .await?;
    }
    for target in targets {
        if !ready.contains_key(&target.name) {
            tracing::warn!(lab_id = %lab_id, node_name = %target.name, timeout_secs = timeout.as_secs(), "Resumed node not ready within timeout");
            report(
                progress,
                format!(
                    "Node {} did not become ready within {} seconds",
                    target.name,
                    timeout.as_secs()
                ),
                StatusKind::Waiting,
            );
        }
    }
    Ok(ready)
}

impl StartOutcome {
    /// What the operation records for the node: a cold-booted node is ready only once it
    /// answers, so `Ready` is recorded for it after the readiness wait.
    fn events(&self) -> &'static [OperationEvent] {
        match self {
            StartOutcome::ColdBooted => &[OperationEvent::Launched],
            StartOutcome::Resumed => &[OperationEvent::Launched, OperationEvent::Ready],
            StartOutcome::AlreadyRunning | StartOutcome::Failed(_) => &[],
        }
    }

    /// The node's result, as reported to the user.
    fn result(&self, node_name: &str) -> NodeActionResult {
        let (success, message) = match self {
            StartOutcome::ColdBooted => (true, "Started".to_string()),
            StartOutcome::Resumed => (true, "Resumed".to_string()),
            StartOutcome::AlreadyRunning => (true, "Already running".to_string()),
            StartOutcome::Failed(reason) => (false, reason.clone()),
        };
        NodeActionResult {
            name: node_name.to_string(),
            success,
            message,
            links: Vec::new(),
        }
    }
}

/// What resume does to a node whose `what` ("domain" or "container") is in `runtime`, or why
/// it cannot start it.
fn start_step(runtime: Runtime, what: &str) -> Result<StartStep, String> {
    match runtime {
        Runtime::Running => Ok(StartStep::Nothing),
        Runtime::Stopped(Halt::Off) => Ok(StartStep::Boot),
        Runtime::Stopped(Halt::Paused) => Ok(StartStep::Resume),
        Runtime::Faulted(word) => Err(format!(
            "Cannot start a node whose {what} is {word}; redeploy it"
        )),
        Runtime::Transitional(word) => Err(format!(
            "Cannot start a node whose {what} is {word}; try again shortly"
        )),
        Runtime::Absent => Err(format!("The node has no {what}; redeploy it")),
    }
}

/// The kind of `node`, from its image, if it may be started: or the result that says why not,
/// for a node with no image, or one whose state does not allow a start.
fn startable_kind(
    node: &data::DbNode,
    node_images: &[data::NodeConfig],
) -> Result<NodeKind, NodeActionResult> {
    let refused = |message: String| NodeActionResult {
        name: node.name.clone(),
        success: false,
        message,
        links: Vec::new(),
    };
    let kind = node_images
        .iter()
        .find(|image| image.id.as_ref() == Some(&node.image))
        .map(|image| image.kind.clone())
        .ok_or_else(|| refused("Node image not found in database".to_string()))?;
    node.state
        .on_operation(OperationEvent::Launched)
        .map_err(|refusal| refused(format!("Cannot start: {refusal}")))?;
    Ok(kind)
}

/// Record what starting each node did, add each node's result to `results`, and return the
/// cold-booted nodes: they have new taps with new ifindexes, and their readiness is awaited.
async fn record_outcomes(
    state: &AppState,
    operation: &LabOperation,
    outcomes: Vec<(StartOutcome, String, Option<RecordId>, NodeKind)>,
    results: &mut Vec<NodeActionResult>,
) -> Vec<(String, NodeKind)> {
    let mut cold_booted = vec![];
    for (outcome, node_name, node_id, kind) in outcomes {
        let mut result = outcome.result(&node_name);
        if let Some(id) = &node_id
            && let Err(error) = record_all(state, operation, id, outcome.events()).await
        {
            tracing::warn!(node = %node_name, error = %error, "Failed to record the started node");
            result.success = false;
            result.message = format!("{}, but not recorded: {error:#}", result.message);
        }
        if outcome == StartOutcome::ColdBooted {
            cold_booted.push((node_name, kind));
        }
        results.push(result);
    }
    cold_booted
}

/// Record `events` for node `node_id`, in order.
async fn record_all(
    state: &AppState,
    operation: &LabOperation,
    node_id: &RecordId,
    events: &[OperationEvent],
) -> Result<()> {
    for event in events {
        node_lifecycle::record(&state.db, operation, node_id, *event).await?;
    }
    Ok(())
}

async fn start_vm(device_name: &str, state: &AppState) -> StartOutcome {
    let qemu = state.qemu.clone();
    let device_name = device_name.to_string();

    match tokio::task::spawn_blocking(move || -> Result<StartOutcome> {
        let conn = qemu.connect().context("Failed to connect to libvirt")?;
        // A domain libvirt cannot find has no runtime object.
        let Ok(domain) = virt::domain::Domain::lookup_by_name(&conn, &device_name) else {
            return Ok(StartOutcome::Failed(
                start_step(Runtime::Absent, "domain")
                    .err()
                    .unwrap_or_default(),
            ));
        };
        let code = match domain.get_state() {
            Ok((code, _reason)) => code,
            Err(e) => return Ok(StartOutcome::Failed(format!("Failed to get state: {e}"))),
        };
        let outcome = match start_step(node_lifecycle::libvirt_runtime(Some(code)), "domain") {
            Ok(StartStep::Boot) => match domain.create() {
                Ok(_) => StartOutcome::ColdBooted,
                Err(e) => StartOutcome::Failed(format!("Failed to start: {e}")),
            },
            Ok(StartStep::Resume) => match domain.resume() {
                Ok(_) => StartOutcome::Resumed,
                Err(e) => StartOutcome::Failed(format!("Failed to resume: {e}")),
            },
            Ok(StartStep::Nothing) => StartOutcome::AlreadyRunning,
            Err(reason) => StartOutcome::Failed(reason),
        };
        Ok(outcome)
    })
    .await
    {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(e)) => StartOutcome::Failed(format!("Libvirt error: {e}")),
        Err(e) => StartOutcome::Failed(format!("Task panicked: {e}")),
    }
}

async fn start_container_node(device_name: &str, state: &AppState) -> StartOutcome {
    // As for VMs, a running node is left alone and a stopped one is started.
    let status = match container::container_status(&state.docker, device_name).await {
        Ok(status) => status,
        Err(e) => return StartOutcome::Failed(format!("Failed to check the container: {e:#}")),
    };
    match start_step(node_lifecycle::container_runtime(status), "container") {
        Ok(StartStep::Resume) => {
            match container::unpause_container(&state.docker, device_name).await {
                Ok(()) => StartOutcome::Resumed,
                Err(e) => StartOutcome::Failed(format!("Failed to unpause: {e:#}")),
            }
        }
        Ok(StartStep::Boot) => match container::start_container(&state.docker, device_name).await {
            Ok(()) => StartOutcome::ColdBooted,
            Err(e) => StartOutcome::Failed(format!("Failed to start: {e:#}")),
        },
        Ok(StartStep::Nothing) => StartOutcome::AlreadyRunning,
        Err(reason) => StartOutcome::Failed(reason),
    }
}

/// Re-attach the links of each cold-booted node, whose ends were made again, and add their
/// states to its result. A link that is not attached does not fail the node.
#[instrument(skip_all, fields(lab_id = %lab.lab_id), level = "debug")]
async fn reattach_cold_booted(
    lab: &ResumedLab<'_>,
    node_images: &[data::NodeConfig],
    cold_booted: &[(String, NodeKind)],
    results: &mut [NodeActionResult],
) {
    if cold_booted.is_empty() {
        return;
    }
    let db = &lab.state.db;
    let lab_links = match LabLinks::load(db, lab.lab_record_id).await {
        Ok(lab_links) => lab_links,
        Err(error) => {
            tracing::error!(lab_id = %lab.lab_id, error = ?error, "Failed to load the lab's links");
            for result in results
                .iter_mut()
                .filter(|result| cold_booted.iter().any(|(name, _)| *name == result.name))
            {
                result.message = format!("{}; links not re-attached: {error:#}", result.message);
            }
            return;
        }
    };
    // Only a started container's veths need the bridges; they are loaded once.
    let bridges = if cold_booted
        .iter()
        .any(|(_, kind)| *kind == NodeKind::Container)
    {
        db::list_bridges(db, lab.lab_record_id)
            .await
            .map_err(|error| format!("{error:#}"))
    } else {
        Ok(vec![])
    };
    let host = KernelHost::new(&lab.state.docker);
    for (name, kind) in cold_booted {
        let Some(node) = lab.nodes.iter().find(|node| node.name == *name) else {
            continue;
        };
        let Some(node_id) = &node.id else {
            continue;
        };
        let image = node_images
            .iter()
            .find(|image| image.id.as_ref() == Some(&node.image));
        let Some(end) = recreated_end(
            &ColdBooted {
                lab_id: lab.lab_id,
                node,
                node_id,
                kind,
                image,
            },
            lab.manifest,
            &lab_links,
            bridges.as_deref(),
        ) else {
            continue;
        };
        let reports = reattach_node_links(&host, &lab_links, node_id, end).await;
        report_links(lab.progress, name, &reports);
        if let Some(result) = results.iter_mut().find(|result| result.name == *name) {
            result.links = reports;
        }
    }
}

/// A node that resume cold-booted.
struct ColdBooted<'a> {
    lab_id: &'a str,
    node: &'a data::DbNode,
    node_id: &'a RecordId,
    kind: &'a NodeKind,
    image: Option<&'a data::NodeConfig>,
}

/// How a cold boot made the node's link ends again, or `None` when it has no ends to re-attach.
///
/// libvirt makes a VM's or a unikernel's taps when it boots the domain. A container started
/// from exited has a new network namespace with none of its veths, so each of its data
/// interfaces is planned again, as redeploy does, with the saved manifest's interface count. A
/// container on no link has Docker networks, which Docker connects again.
fn recreated_end(
    node: &ColdBooted<'_>,
    manifest: Option<&Manifest>,
    lab_links: &LabLinks,
    bridges: Result<&[DbBridge], &String>,
) -> Option<RecreatedEnd> {
    match node.kind {
        NodeKind::VirtualMachine | NodeKind::Unikernel => Some(RecreatedEnd::Domain),
        NodeKind::Container => {
            let links = node_links(lab_links.links(), node.node_id);
            if links.is_empty() {
                return None;
            }
            Some(
                container_end(node, manifest, &links, bridges)
                    .unwrap_or_else(|error| RecreatedEnd::Unplanned(format!("{error:#}"))),
            )
        }
    }
}

/// The started container's veths, planned from its links and bridges in the database and its
/// data interface count.
fn container_end(
    node: &ColdBooted<'_>,
    manifest: Option<&Manifest>,
    links: &[&DbLink],
    bridges: Result<&[DbBridge], &String>,
) -> Result<RecreatedEnd> {
    let name = node.node.name.as_str();
    let image = node
        .image
        .ok_or_else(|| anyhow!("Node image of {name} not found"))?;
    let bridges = bridges.map_err(|error| anyhow!("Failed to list the lab's bridges: {error}"))?;
    let manifest_count = manifest
        .and_then(|manifest| manifest.nodes.iter().find(|n| n.name == name))
        .and_then(|n| n.interfaces.data_count);
    let count = effective_data_interface_count(name, manifest_count, image)?;
    let veths = container_veths(
        &ContainerNode {
            lab_id: node.lab_id,
            name,
            index: node.node.index,
            model: image.model,
            record: node.node_id,
        },
        links,
        &node_bridge_interfaces(bridges, node.node_id),
        data_interface_indices(image, count, name)?,
    )?;
    Ok(RecreatedEnd::Container {
        container: format!("{name}-{}", node.lab_id),
        veths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeState;

    #[test]
    fn resume_boots_resumes_or_leaves_a_node_by_its_runtime() {
        for what in ["domain", "container"] {
            assert_eq!(
                start_step(Runtime::Stopped(Halt::Off), what),
                Ok(StartStep::Boot)
            );
            assert_eq!(
                start_step(Runtime::Stopped(Halt::Paused), what),
                Ok(StartStep::Resume)
            );
            assert_eq!(start_step(Runtime::Running, what), Ok(StartStep::Nothing));
        }
    }

    #[test]
    fn resume_refuses_a_crashed_domain_or_a_dead_container() {
        // M1.4: a crashed domain used to be reported "Already running" and recorded `Running`.
        let crashed = start_step(Runtime::Faulted("crashed"), "domain").unwrap_err();
        assert!(crashed.contains("domain is crashed"), "{crashed}");
        assert!(crashed.contains("redeploy"), "{crashed}");
        // M4.1: a dead container is refused, and the scanner shows it failed, not stopped.
        let dead = start_step(Runtime::Faulted("dead"), "container").unwrap_err();
        assert!(dead.contains("container is dead"), "{dead}");
        let shutting = start_step(Runtime::Transitional("shutting down"), "domain").unwrap_err();
        assert!(shutting.contains("shutting down"), "{shutting}");
        let gone = start_step(Runtime::Absent, "domain").unwrap_err();
        assert!(gone.contains("no domain"), "{gone}");
    }

    #[test]
    fn a_cold_boot_is_told_by_the_outcome_not_its_message() {
        // M8.2: resume used to compare the message with "Started" to find cold boots.
        assert_eq!(
            StartOutcome::ColdBooted.events(),
            [OperationEvent::Launched]
        );
        assert_eq!(
            StartOutcome::Resumed.events(),
            [OperationEvent::Launched, OperationEvent::Ready]
        );
        assert_eq!(StartOutcome::AlreadyRunning.events(), []);
        assert_eq!(StartOutcome::Failed("no".to_string()).events(), []);
        let result = StartOutcome::ColdBooted.result("r1");
        assert!(result.success);
        assert!(!StartOutcome::Failed("no".to_string()).result("r1").success);
    }

    use shared::konst::TALOS_API_PORT;

    fn manifest(text: &str) -> Manifest {
        toml::from_str(text).unwrap()
    }

    fn db_node(name: &str, ip: &str) -> data::DbNode {
        data::DbNode {
            id: None,
            name: name.to_string(),
            image: RecordId::new("node_image", "x"),
            index: 0,
            lab: RecordId::new("lab", "x"),
            mgmt_ipv4: Some(ip.to_string()),
            mgmt_ipv6: None,
            mgmt_mac: None,
            state: NodeState::Running,
            cpu_count: None,
            memory: None,
            vnc_port: None,
            ssh_host_key: None,
            cpu_pinning: None,
            qemu_thread_cpus: None,
        }
    }

    #[test]
    fn only_cold_booted_guests_are_checked() {
        assert_eq!(
            readiness_check(&NodeKind::VirtualMachine, true, None),
            Some(ReadinessCheck::Port(SSH_PORT))
        );
        assert_eq!(
            readiness_check(&NodeKind::VirtualMachine, false, None),
            None
        );
        assert_eq!(readiness_check(&NodeKind::Container, true, None), None);
        assert_eq!(
            readiness_check(&NodeKind::Unikernel, true, None),
            Some(ReadinessCheck::Unikernel { ready_port: None })
        );
    }

    #[test]
    fn manifest_readiness_settings_are_honoured() {
        let lab = manifest(
            r#"
name = "lab"
ready_timeout = 120
nodes = [
  { name = "skip", model = "ubuntu_linux", ready_check = { skip = true } },
  { name = "uk", model = "generic_unikernel", ready_check = { port = 8080 } },
  { name = "vm", model = "ubuntu_linux" },
]
"#,
        );
        assert_eq!(ready_timeout(Some(&lab)), Duration::from_secs(120));
        assert_eq!(ready_timeout(None), Duration::from_secs(READINESS_TIMEOUT));
        let skip = lab.nodes.iter().find(|node| node.name == "skip");
        assert_eq!(readiness_check(&NodeKind::VirtualMachine, true, skip), None);
        let unikernel = lab.nodes.iter().find(|node| node.name == "uk");
        assert_eq!(
            readiness_check(&NodeKind::Unikernel, true, unikernel),
            Some(ReadinessCheck::Unikernel {
                ready_port: Some(8080)
            })
        );
    }

    #[test]
    fn virtual_machines_are_checked_on_their_model_port_or_manifest_port() {
        let lab = manifest(
            r#"
name = "lab"
nodes = [
  { name = "talos", model = "talos_linux" },
  { name = "talos-custom", model = "talos_linux", ready_check = { port = 6443 } },
  { name = "vm", model = "ubuntu_linux" },
]
"#,
        );
        for (name, port) in [
            ("talos", TALOS_API_PORT),
            ("talos-custom", 6443),
            ("vm", SSH_PORT),
        ] {
            let node = lab.nodes.iter().find(|node| node.name == name);
            assert_eq!(
                readiness_check(&NodeKind::VirtualMachine, true, node),
                Some(ReadinessCheck::Port(port)),
                "{name}"
            );
        }
    }

    #[test]
    fn targets_cover_cold_booted_nodes_with_their_addresses() {
        let lab = manifest(
            r#"
name = "lab"
nodes = [
  { name = "skip", model = "ubuntu_linux", ready_check = { skip = true } },
  { name = "vm", model = "ubuntu_linux" },
  { name = "ct", model = "nokia_srlinux" },
]
"#,
        );
        let nodes = vec![
            db_node("skip", "10.0.0.10"),
            db_node("vm", "10.0.0.11"),
            db_node("ct", "10.0.0.12"),
        ];
        let started = vec![
            ("skip".to_string(), NodeKind::VirtualMachine),
            ("vm".to_string(), NodeKind::VirtualMachine),
            ("ct".to_string(), NodeKind::Container),
        ];
        let targets = readiness_targets("abcd1234", &nodes, &started, Some(&lab));
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "vm");
        assert_eq!(targets[0].device_name, "vm-abcd1234");
        assert_eq!(targets[0].mgmt_ipv4.as_deref(), Some("10.0.0.11"));
        assert_eq!(targets[0].check, ReadinessCheck::Port(SSH_PORT));
    }

    #[test]
    fn readiness_is_reported_in_the_node_result() {
        assert_eq!(
            readiness_message(true, Duration::from_secs(600)),
            "Started and ready"
        );
        assert_eq!(
            readiness_message(false, Duration::from_secs(600)),
            "Started, but not ready within 600 seconds"
        );
    }

    fn linked_lab() -> LabLinks {
        let link = |index: u16, a: &str, b: &str| DbLink {
            id: None,
            index,
            kind: data::BridgeKind::P2p,
            node_a: RecordId::new("node", a),
            node_b: RecordId::new("node", b),
            int_a: "eth1".to_string(),
            int_b: "eth1".to_string(),
            lab: RecordId::new("lab", "x"),
            tap_a: format!("tpa{index}-lab1"),
            tap_b: format!("tpb{index}-lab1"),
            impairment_a_to_b: data::LinkImpairment::default(),
            impairment_b_to_a: data::LinkImpairment::default(),
        };
        LabLinks::new(vec![link(0, "vm1", "uk1"), link(1, "vm1", "ctr1")], &[])
    }

    fn end_of(
        name: &str,
        kind: &NodeKind,
        image: Option<&data::NodeConfig>,
    ) -> Option<RecreatedEnd> {
        let mut node = db_node(name, "172.31.1.5");
        node.index = 3;
        let node_id = RecordId::new("node", name);
        let bridges: Vec<DbBridge> = vec![];
        recreated_end(
            &ColdBooted {
                lab_id: "lab1",
                node: &node,
                node_id: &node_id,
                kind,
                image,
            },
            Some(&manifest(
                "name = \"l\"\n[[nodes]]\nname = \"ctr1\"\nmodel = \"frr_linux\"\n[nodes.interfaces]\ndata_count = 2\n",
            )),
            &linked_lab(),
            Ok(&bridges),
        )
    }

    #[test]
    fn a_cold_booted_unikernel_has_its_links_reattached() {
        // M8.1: only cold-booted VMs used to be re-attached; a unikernel's new tap was left
        // without a redirect, and its peer's pointed at the old one.
        assert!(matches!(
            end_of("uk1", &NodeKind::Unikernel, None),
            Some(RecreatedEnd::Domain)
        ));
        assert!(matches!(
            end_of("vm1", &NodeKind::VirtualMachine, None),
            Some(RecreatedEnd::Domain)
        ));
    }

    #[test]
    fn a_started_container_has_its_veths_made_again() {
        // M25.2: a container started from exited has a new network namespace without its
        // veths; resume used to leave it so.
        let image = data::NodeConfig::get_model(data::NodeModel::FrrLinux);
        let Some(RecreatedEnd::Container { container, veths }) =
            end_of("ctr1", &NodeKind::Container, Some(&image))
        else {
            panic!("a started container on a link has its ends made again");
        };
        assert_eq!(container, "ctr1-lab1");
        let host_veths: Vec<&str> = veths.iter().map(|v| v.host_veth.as_str()).collect();
        // Its link's end, and its other data interface, disabled, from the manifest's count.
        assert_eq!(host_veths, vec!["tpb1-lab1", "cd3i2-lab1"]);
    }

    #[test]
    fn a_started_container_on_no_link_keeps_its_docker_networks() {
        let image = data::NodeConfig::get_model(data::NodeModel::FrrLinux);
        assert!(end_of("ctr9", &NodeKind::Container, Some(&image)).is_none());
    }

    #[test]
    fn a_started_container_that_cannot_be_planned_fails_its_links() {
        assert!(matches!(
            end_of("ctr1", &NodeKind::Container, None),
            Some(RecreatedEnd::Unplanned(_))
        ));
    }
}
