use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use libvirt::ConsoleSockets;
use opentelemetry::KeyValue;
use shared::data::{
    self, DbLab, Halt, LabNodeActionResponse, ModelKind, NodeActionResult, NodeKind,
    OperationEvent, RecordId, Runtime, StatusKind, TailnetAuthKey, TailnetStatus,
};
use shared::konst::READINESS_TIMEOUT;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use topology::Manifest;

use tracing::instrument;

use super::admission::{self, Admission};
use super::console;
use super::lab_definition::stored_definition;
use super::lab_lifecycle;
use super::lab_nodes::load_target_nodes;
use super::lab_operations::{self, LabOperation};
use super::node_lifecycle::{self, query_runtimes};
use super::node_ops::{self, VmReadiness};
use super::p2p_links::{
    KernelHost, LabLinks, PlannedNode, follow_started_links, reattach_node_links, recreated_end,
    report_links,
};
use super::progress::ProgressSender;
use super::ssh_config;
use crate::auth::context::AuthContext;
use crate::auth::policy::{require_lab_access, require_lab_starter};
use crate::daemon::state::AppState;
use crate::konst::READINESS_SLEEP;

/// How a node started by resume shows that it is ready, as `up` checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReadinessCheck {
    /// A virtual machine or container accepting connections on `port`, checked as `up` checks
    /// VMs (`node_ops::check_vm_ready`), its SSH server showing Sherpa's ready marker when
    /// `sshd_marker` is set.
    Port {
        port: u16,
        sshd_marker: bool,
    },
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
/// Cold-booted VMs and unikernels, and cold-booted containers with a ready port, are then
/// polled for readiness like `up` does, on the readiness check stored with each node
/// (`DbNode::ready_check`), until the `ready_timeout` of the lab's stored definition. A node that does not become
/// ready within the timeout is reported in its result message; it is not an error, as with
/// `up`.
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
    let admission = admit_stopped_nodes(state, ctx, lab_id, &target_nodes).await?;
    if let Some(progress) = progress {
        for warning in admission.shared_cpus() {
            let _ = progress.send_status(format!("Warning: {warning}"), StatusKind::Info);
        }
    }

    let tailscale =
        resume_tailscale(state, &db_lab, node_name, tailscale_auth_key, progress).await?;
    // A lab started before consoles moved behind sherpad has no console folder yet.
    console::create_console_dir(lab_id)?;

    // Build futures for all node operations concurrently
    let mut futures = Vec::new();
    let mut immediate_results = Vec::new();

    for node in &target_nodes {
        let device_name = format!("{}-{}", node.name, lab_id);
        let (kind, model) = match startable_image(node, &node_images) {
            Ok(image) => (image.kind(), image.model),
            Err(refused) => {
                immediate_results.push(refused);
                continue;
            }
        };

        let node_name = node.name.clone();
        let node_id = node.id.clone();
        let state = state.clone();
        let sockets = console::console_sockets(lab_id, node.index);

        futures.push(async move {
            let outcome = match (kind, sockets) {
                (NodeKind::VirtualMachine | NodeKind::Unikernel, Ok(sockets)) => {
                    start_vm(&device_name, sockets, &state).await
                }
                (NodeKind::VirtualMachine | NodeKind::Unikernel, Err(error)) => {
                    StartOutcome::Failed(format!("{error:#}"))
                }
                (NodeKind::Container, _) => start_container_node(&device_name, model, &state).await,
            };

            (outcome, node_name, node_id, kind)
        });
    }

    // Run all operations concurrently
    let concurrent_results = join_all(futures).await;

    // Booted or unpaused: their links' states and carrier are decided once they are re-attached.
    let started: HashSet<RecordId> = concurrent_results
        .iter()
        .filter(|(outcome, ..)| matches!(outcome, StartOutcome::ColdBooted | StartOutcome::Resumed))
        .filter_map(|(_, _, node_id, _)| node_id.clone())
        .collect();
    let mut results = immediate_results;
    let cold_booted = record_outcomes(state, &operation, concurrent_results, &mut results).await;
    let definition = stored_definition(&state.db, &lab_record_id, lab_id).await;
    let lab = ResumedLab {
        lab_id,
        lab_record_id: &lab_record_id,
        nodes: &target_nodes,
        definition: definition.as_ref(),
        state,
        progress,
        operation: &operation,
    };

    // A cold boot makes a node's link ends again, so its links are re-attached before its
    // readiness is awaited, as `up` does.
    reattach_cold_booted(&lab, &node_images, &cold_booted, &mut results).await;
    // An unpaused node made no ends again, and a booted one's re-attach takes a paused peer as
    // up: the links of both are decided now, as the next scan would, and their carrier set.
    follow_started(&lab, &started).await;
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
    requester: &AuthContext,
    lab_id: &str,
    target_nodes: &[data::DbNode],
) -> Result<Admission> {
    let conn = state
        .qemu
        .connect()
        .context("Failed to connect to libvirt")?;
    admission::admit_stopped_nodes(
        &state.db,
        &conn,
        &state.docker,
        requester,
        state.config.host_memory,
        lab_id,
        target_nodes,
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
    /// The lab's stored definition, if it has one: its `ready_timeout`, and its containers'
    /// data interface counts.
    definition: Option<&'a Manifest>,
    state: &'a AppState,
    progress: Option<&'a ProgressSender>,
    operation: &'a LabOperation,
}

/// Wait for the cold-booted VMs, unikernels and containers with a ready port to become ready,
/// as `up` does, on each node's stored readiness check and the stored definition's ready
/// timeout. Each one's result says whether it became ready, and the host keys it offers are
/// recorded again. Returns the cold-booted nodes that are ready: those that
/// answered, and those with no check, as a container without a ready port is ready once
/// running.
#[instrument(skip_all, fields(lab_id = %lab.lab_id), level = "debug")]
async fn wait_for_cold_booted(
    lab: &ResumedLab<'_>,
    cold_booted: &[(String, NodeKind)],
    results: &mut [NodeActionResult],
) -> Result<HashSet<String>> {
    let ResumedLab {
        lab_id,
        lab_record_id: _,
        nodes,
        definition,
        state,
        progress,
        operation,
    } = *lab;
    let targets = readiness_targets(lab_id, nodes, cold_booted);
    let mut ready_nodes: HashSet<String> = cold_booted
        .iter()
        .map(|(name, _)| name.clone())
        .filter(|name| !targets.iter().any(|target| &target.name == name))
        .collect();
    if targets.is_empty() {
        return Ok(ready_nodes);
    }
    let timeout = ready_timeout(definition);
    let ready = wait_for_readiness(lab_id, &targets, timeout, state, progress, operation).await?;
    for result in results.iter_mut() {
        if targets.iter().any(|target| target.name == result.name) {
            result.message = readiness_message(ready.contains_key(&result.name), timeout);
        }
    }
    // A node may make a new host key when it boots, so record the keys again.
    if let Err(error) = save_host_keys(lab_id, &ready, nodes, state).await {
        tracing::warn!(lab_id = %lab_id, error = ?error, "Failed to save resumed nodes' host keys");
    }
    ready_nodes.extend(ready.into_keys());
    Ok(ready_nodes)
}

/// The lab's `ready_timeout`, from its stored definition; the default without one.
fn ready_timeout(definition: Option<&Manifest>) -> Duration {
    let seconds = match definition {
        Some(definition) => definition.ready_timeout.unwrap_or(READINESS_TIMEOUT),
        None => READINESS_TIMEOUT,
    };
    Duration::from_secs(seconds)
}

/// Decide how a started node proves readiness, from the check stored with it. Paused nodes
/// resume an already booted guest, so only cold-booted nodes are checked, unless their check is
/// skipped: VMs and containers on their stored port, as without one they are ready once
/// running, and unikernels by their domain, and their stored port when they have one.
fn readiness_check(
    kind: NodeKind,
    cold_booted: bool,
    ready_check: &data::ReadyCheck,
) -> Option<ReadinessCheck> {
    if !cold_booted || ready_check.skip {
        return None;
    }
    match kind {
        NodeKind::Unikernel => Some(ReadinessCheck::Unikernel {
            ready_port: ready_check.port,
        }),
        NodeKind::VirtualMachine | NodeKind::Container => {
            ready_check.port.map(|port| ReadinessCheck::Port {
                port,
                sshd_marker: ready_check.sshd_marker,
            })
        }
    }
}

fn readiness_targets(
    lab_id: &str,
    nodes: &[data::DbNode],
    cold_booted: &[(String, NodeKind)],
) -> Vec<ReadinessTarget> {
    let mut targets = vec![];
    for (name, kind) in cold_booted {
        let Some(node) = nodes.iter().find(|node| &node.name == name) else {
            tracing::debug!(lab_id = %lab_id, node_name = %name, "No record for cold-booted node");
            continue;
        };
        let Some(check) = readiness_check(*kind, true, &node.ready_check) else {
            tracing::debug!(lab_id = %lab_id, node_name = %name, "No readiness check for node");
            continue;
        };
        targets.push(ReadinessTarget {
            name: name.clone(),
            device_name: format!("{name}-{lab_id}"),
            mgmt_ipv4: node.mgmt_ipv4.clone(),
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
        (ReadinessCheck::Port { port, sshd_marker }, Some(ip)) => {
            node_ops::check_vm_ready(ip, *port, *sshd_marker).await
        }
        (ReadinessCheck::Port { .. }, None) => Ok(VmReadiness::NotReady),
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

/// Record the host keys that resumed nodes offered. Nodes ready without an SSH handshake keep
/// their recorded keys.
#[instrument(skip(ready, nodes, state), level = "debug")]
async fn save_host_keys(
    lab_id: &str,
    ready: &HashMap<String, Option<String>>,
    nodes: &[data::DbNode],
    state: &AppState,
) -> Result<()> {
    for node in nodes {
        let Some(Some(host_key)) = ready.get(&node.name) else {
            tracing::trace!(node_name = %node.name, "No host key offered");
            continue;
        };
        ssh_config::record_node_host_key(&state.db, node, host_key).await?;
    }
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

/// The image of `node`, if it may be started: or the result that says why not, for a node with
/// no image, or one whose state does not allow a start.
fn startable_image<'a>(
    node: &data::DbNode,
    node_images: &'a [data::NodeConfig],
) -> Result<&'a data::NodeConfig, NodeActionResult> {
    let refused = |message: String| NodeActionResult {
        name: node.name.clone(),
        success: false,
        message,
        links: Vec::new(),
    };
    let image = node_images
        .iter()
        .find(|image| image.id.as_ref() == Some(&node.image))
        .ok_or_else(|| refused("Node image not found in database".to_string()))?;
    node.state
        .on_operation(OperationEvent::Launched)
        .map_err(|refusal| refused(format!("Cannot start: {refusal}")))?;
    Ok(image)
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

/// Start a shut-off domain, or unpause a paused one. A domain defined before consoles moved
/// behind sherpad has its serial and VNC consoles moved to `sockets` before it boots.
async fn start_vm(device_name: &str, sockets: ConsoleSockets, state: &AppState) -> StartOutcome {
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
            Ok(StartStep::Boot) => {
                match libvirt::move_consoles_to_sockets(&conn, &device_name, &sockets) {
                    Ok(_) => match domain.create() {
                        Ok(_) => StartOutcome::ColdBooted,
                        Err(e) => StartOutcome::Failed(format!("Failed to start: {e}")),
                    },
                    Err(e) => StartOutcome::Failed(format!(
                        "Failed to move the consoles to their sockets: {e:#}"
                    )),
                }
            }
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

/// Start a stopped container, or unpause a paused one. A cold-booted container has a new
/// management veth, which some models need checksum offload off on.
async fn start_container_node(
    device_name: &str,
    model: data::NodeModel,
    state: &AppState,
) -> StartOutcome {
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
        Ok(StartStep::Boot) => {
            let model_kind = match ModelKind::new(model, Some(NodeKind::Container)) {
                Ok(model_kind) => model_kind,
                Err(e) => return StartOutcome::Failed(format!("{e:#}")),
            };
            if let Err(e) = container::start_container(&state.docker, device_name).await {
                return StartOutcome::Failed(format!("Failed to start: {e:#}"));
            }
            match node_ops::finish_management_checksums(
                &KernelHost::new(&state.docker, &state.qemu),
                device_name,
                model_kind,
            )
            .await
            {
                Ok(_) => StartOutcome::ColdBooted,
                Err(e) => StartOutcome::Failed(format!("{e:#}")),
            }
        }
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
    let lab_links = match LabLinks::load(db, lab.lab_id, lab.lab_record_id).await {
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
    let host = KernelHost::new(&lab.state.docker, &lab.state.qemu);
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
            &PlannedNode {
                lab_id: lab.lab_id,
                node,
                node_id,
                kind,
                image,
            },
            lab.definition,
            &lab_links,
            bridges.as_deref(),
        ) else {
            continue;
        };
        let outcomes = reattach_node_links(&host, &lab_links, node_id, end).await;
        let reports = report_links(
            &host,
            &lab_links,
            node_id,
            lab.progress,
            &lab.state.link_states,
            outcomes,
        )
        .await;
        if let Some(result) = results.iter_mut().find(|result| result.name == *name) {
            result.links = reports;
        }
    }
}

/// Decide the links of the `started` nodes as the next scan would, and set their carrier at
/// once: an unpaused node's peers get their link back, and a node booted beside a paused peer
/// has its link taken down. See `p2p_links::started`.
#[instrument(skip_all, fields(lab_id = %lab.lab_id), level = "debug")]
async fn follow_started(lab: &ResumedLab<'_>, started: &HashSet<RecordId>) {
    if started.is_empty() {
        return;
    }
    let lab_links = match LabLinks::load(&lab.state.db, lab.lab_id, lab.lab_record_id).await {
        Ok(lab_links) => lab_links,
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "Failed to load the lab's links; their carrier is left to the scan");
            return;
        }
    };
    let runtimes = query_runtimes(&lab.state.docker, &lab.state.qemu).await;
    follow_started_links(
        &KernelHost::new(&lab.state.docker, &lab.state.qemu),
        &lab_links,
        started,
        &runtimes,
        &lab.state.link_states,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::lab_definition::tests::{lab_with, store};
    use shared::data::{NodeModel, NodeState, ReadyCheck};
    use shared::konst::SSH_PORT;

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

    fn db_node(name: &str, ip: &str, port: Option<u16>, skip: bool) -> data::DbNode {
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
            ssh_host_key: None,
            ready_check: stored(port, skip),
            ssh_username: None,
        }
    }

    fn stored(port: Option<u16>, skip: bool) -> ReadyCheck {
        ReadyCheck {
            port,
            skip,
            sshd_marker: false,
        }
    }

    /// A check on `port` without Sherpa's SSH ready marker.
    fn on_port(port: u16) -> ReadinessCheck {
        ReadinessCheck::Port {
            port,
            sshd_marker: false,
        }
    }

    #[test]
    fn only_cold_booted_guests_are_checked() {
        let ssh = stored(Some(SSH_PORT), false);
        assert_eq!(
            readiness_check(NodeKind::VirtualMachine, true, &ssh),
            Some(on_port(SSH_PORT))
        );
        assert_eq!(readiness_check(NodeKind::VirtualMachine, false, &ssh), None);
        let none = stored(None, false);
        assert_eq!(readiness_check(NodeKind::Container, true, &none), None);
        assert_eq!(readiness_check(NodeKind::Container, false, &none), None);
        assert_eq!(
            readiness_check(NodeKind::Unikernel, true, &none),
            Some(ReadinessCheck::Unikernel { ready_port: None })
        );
    }

    #[test]
    fn the_stored_readiness_check_is_honoured() {
        assert_eq!(
            readiness_check(
                NodeKind::VirtualMachine,
                true,
                &stored(Some(SSH_PORT), true)
            ),
            None
        );
        assert_eq!(
            readiness_check(NodeKind::Unikernel, true, &stored(Some(8080), false)),
            Some(ReadinessCheck::Unikernel {
                ready_port: Some(8080)
            })
        );
        assert_eq!(
            readiness_check(NodeKind::Unikernel, true, &stored(Some(8080), true)),
            None
        );
    }

    #[tokio::test]
    async fn the_stored_definition_gives_only_the_ready_timeout() {
        let (db, lab) = lab_with(&[("vm", NodeModel::UbuntuLinux)]).await;
        let lab_id = lab.id.clone().unwrap();
        // No stored definition: the default.
        let none = stored_definition(&db, &lab_id, "lab-0001").await;
        assert_eq!(ready_timeout(none.as_ref()), Duration::from_secs(600));
        store(
            &db,
            &lab,
            "name = \"core\"\nready_timeout = 120\n\
             nodes = [{ name = \"vm\", model = \"ubuntu_linux\", ready_check = { port = 8443 } }]\n",
        )
        .await;
        let definition = stored_definition(&db, &lab_id, "lab-0001").await;
        assert_eq!(ready_timeout(definition.as_ref()), Duration::from_secs(120));
    }

    #[test]
    fn a_windows_node_waits_for_sherpa_s_ready_marker_as_up_does() {
        let windows = ReadyCheck {
            sshd_marker: true,
            ..stored(Some(SSH_PORT), false)
        };
        assert_eq!(
            readiness_check(NodeKind::VirtualMachine, true, &windows),
            Some(ReadinessCheck::Port {
                port: SSH_PORT,
                sshd_marker: true
            })
        );
    }

    #[test]
    fn a_cold_booted_container_is_checked_on_its_stored_port() {
        let web = stored(Some(80), false);
        assert_eq!(
            readiness_check(NodeKind::Container, true, &web),
            Some(on_port(80))
        );
        // An unpaused container resumes a process that already answered.
        assert_eq!(readiness_check(NodeKind::Container, false, &web), None);
        assert_eq!(
            readiness_check(NodeKind::Container, true, &stored(Some(80), true)),
            None
        );
        assert_eq!(
            readiness_check(NodeKind::Container, true, &stored(None, false)),
            None
        );
    }

    #[test]
    fn targets_cover_cold_booted_nodes_with_their_addresses() {
        let nodes = vec![
            db_node("skip", "10.0.0.10", Some(SSH_PORT), true),
            db_node("vm", "10.0.0.11", Some(8443), false),
            db_node("ct", "10.0.0.12", None, false),
            db_node("web", "10.0.0.13", Some(80), false),
        ];
        let started = vec![
            ("skip".to_string(), NodeKind::VirtualMachine),
            ("vm".to_string(), NodeKind::VirtualMachine),
            ("ct".to_string(), NodeKind::Container),
            ("web".to_string(), NodeKind::Container),
            ("gone".to_string(), NodeKind::Container),
        ];
        let targets = readiness_targets("abcd1234", &nodes, &started);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].name, "vm");
        assert_eq!(targets[0].device_name, "vm-abcd1234");
        assert_eq!(targets[0].mgmt_ipv4.as_deref(), Some("10.0.0.11"));
        assert_eq!(targets[0].check, on_port(8443));
        assert_eq!(targets[1].name, "web");
        assert_eq!(targets[1].check, on_port(80));
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
}
