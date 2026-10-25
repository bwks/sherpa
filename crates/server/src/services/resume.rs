use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use opentelemetry::KeyValue;
use shared::data::{
    self, LabNodeActionResponse, NodeActionResult, NodeKind, NodeState, RecordId, StatusKind,
};
use shared::konst::{READINESS_SLEEP, READINESS_TIMEOUT, SHERPA_LABS_PATH, SSH_PORT};
use shared::util::load_saved_manifest;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use topology::Manifest;
use virt::sys::{VIR_DOMAIN_PAUSED, VIR_DOMAIN_SHUTOFF};

use tracing::instrument;

use super::lab_operations::{self, LabOperation};
use super::node_ops;
use super::progress::ProgressSender;
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
/// Containers: calls `docker start`.
/// DB node state is updated to `Running` on success.
///
/// Cold-booted VMs and unikernels are then polled for readiness like `up` does, using the
/// lab's saved manifest for `ready_timeout`, `skip_ready_check` and `ready_port`. A node that
/// does not become ready within the timeout is reported in its result message; it is not an
/// error, as with `up`.
///
/// The caller must own the lab or be an admin, and may not be the built-in `admin` account.
#[instrument(skip(ctx, state, progress), fields(username = %ctx.username, %lab_id))]
pub async fn start_lab_nodes(
    ctx: &AuthContext,
    lab_id: &str,
    node_name: Option<&str>,
    state: &AppState,
    progress: Option<&ProgressSender>,
) -> Result<LabNodeActionResponse> {
    let start = Instant::now();

    require_lab_starter(ctx)?;
    let db_lab = require_lab_access(ctx, &state.db, lab_id).await?;
    // Destroying the lab stops this operation; see `lab_operations`.
    let operation = lab_operations::begin(lab_id).await;

    let tailscale = if node_name.is_none() {
        super::tailscale::resume(state, &db_lab).await?
    } else {
        super::tailscale::inspect(state, &db_lab).await
    };

    let lab_record_id = db_lab.id.ok_or_else(|| anyhow!("Lab missing record ID"))?;

    // Get all nodes for this lab
    let db_nodes = db::list_nodes_by_lab(&state.db, lab_record_id.clone())
        .await
        .context("Failed to list nodes for lab")?;

    // Filter by node_name if provided
    let target_nodes: Vec<_> = if let Some(name) = node_name {
        db_nodes.into_iter().filter(|n| n.name == name).collect()
    } else {
        db_nodes
    };

    if let Some(name) = node_name
        && target_nodes.is_empty()
    {
        return Err(anyhow!("Node '{}' not found in lab '{}'", name, lab_id));
    }

    // Batch-fetch node images to determine kind (VM vs Container)
    let mut image_ids: Vec<RecordId> = target_nodes.iter().map(|n| n.image.clone()).collect();
    image_ids.dedup();

    let node_images = db::list_node_images_by_ids(&state.db, image_ids)
        .await
        .context("Failed to batch fetch node images")?;

    // Build futures for all node operations concurrently
    let mut futures = Vec::new();
    let mut immediate_results = Vec::new();

    for node in &target_nodes {
        let device_name = format!("{}-{}", node.name, lab_id);

        // Determine node kind from image
        let node_image = node_images
            .iter()
            .find(|img| img.id.as_ref() == Some(&node.image));

        let kind = match node_image {
            Some(img) => img.kind.clone(),
            None => {
                immediate_results.push(NodeActionResult {
                    name: node.name.clone(),
                    success: false,
                    message: "Node image not found in database".to_string(),
                });
                continue;
            }
        };

        let node_name = node.name.clone();
        let node_id = node.id.clone();
        let state = state.clone();

        let kind_clone = kind.clone();
        futures.push(async move {
            let result = match kind {
                NodeKind::VirtualMachine | NodeKind::Unikernel => {
                    start_vm(&device_name, &node_name, &state).await
                }
                NodeKind::Container => start_container_node(&device_name, &node_name, &state).await,
            };

            (result, node_name, node_id, kind_clone)
        });
    }

    // Run all operations concurrently
    let concurrent_results = join_all(futures).await;

    // Collect results and update DB state
    let mut results = immediate_results;
    let mut cold_booted_vms: Vec<String> = vec![];
    let mut cold_booted: Vec<(String, NodeKind)> = vec![];
    for (result, node_name, node_id, kind) in concurrent_results {
        if result.success
            && let Some(id) = node_id
            && let Err(e) = db::update_node_state(&state.db, id, NodeState::Running).await
        {
            tracing::warn!("Failed to update DB state for node '{}': {}", node_name, e);
        }
        // Track VMs that were cold-booted (new tap devices with new ifindexes)
        if result.success && result.message == "Started" && matches!(kind, NodeKind::VirtualMachine)
        {
            cold_booted_vms.push(node_name.clone());
        }
        if result.success && result.message == "Started" {
            cold_booted.push((node_name.clone(), kind));
        }
        results.push(result);
    }

    // Re-attach eBPF redirect on P2p links for cold-booted VMs.
    // Cold boot creates new tap devices, so the eBPF programs need re-attaching
    // on both sides (new VM tap + peer's stale redirect).
    if !cold_booted_vms.is_empty()
        && let Err(e) =
            reattach_p2p_ebpf_for_nodes(lab_id, &cold_booted_vms, &lab_record_id, state).await
    {
        tracing::error!(
            lab_id = %lab_id,
            error = ?e,
            "Failed to re-attach eBPF redirect after VM cold boot"
        );
    }

    // Wait for cold-booted nodes after their links are restored, as `up` does.
    let manifest = load_lab_manifest(lab_id);
    let targets = readiness_targets(lab_id, &target_nodes, &cold_booted, manifest.as_ref());
    if !targets.is_empty() {
        let timeout = ready_timeout(manifest.as_ref());
        let ready =
            wait_for_readiness(lab_id, &targets, timeout, state, progress, &operation).await?;
        for result in results.iter_mut() {
            if targets.iter().any(|target| target.name == result.name) {
                result.message = readiness_message(ready.contains(&result.name), timeout);
            }
        }
    }

    state.metrics.operation_duration.record(
        start.elapsed().as_secs_f64(),
        &[KeyValue::new("operation.type", "resume")],
    );

    Ok(LabNodeActionResponse { results, tailscale })
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
        Some(node) => (node.skip_ready_check.unwrap_or(false), node.ready_port),
        None => (false, None),
    };
    if !cold_booted || skip {
        return None;
    }
    match kind {
        // Without a saved manifest the model is unknown, so fall back to SSH as before.
        NodeKind::VirtualMachine => Some(ReadinessCheck::Port(match manifest_node {
            Some(node) => node_ops::vm_ready_port(&node.model, ready_port),
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
fn probe(target: &ReadinessTarget, state: &AppState) -> bool {
    let outcome = match (&target.check, target.mgmt_ipv4.as_deref()) {
        (ReadinessCheck::Port(port), Some(ip)) => node_ops::check_node_port_ready(ip, *port),
        (ReadinessCheck::Port(_), None) => Ok(false),
        (ReadinessCheck::Unikernel { ready_port }, ip) => match state.qemu.connect() {
            Ok(conn) => {
                node_ops::check_unikernel_ready(&conn, &target.device_name, *ready_port, ip)
            }
            Err(error) => Err(error),
        },
    };
    match outcome {
        Ok(ready) => ready,
        Err(error) => {
            tracing::debug!(node_name = %target.name, error = ?error, "Readiness probe failed");
            false
        }
    }
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
) -> Result<HashSet<String>> {
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
    let mut ready = HashSet::new();
    loop {
        for target in targets {
            if ready.contains(&target.name) || !probe(target, state) {
                continue;
            }
            tracing::info!(lab_id = %lab_id, node_name = %target.name, "Resumed node ready");
            report(
                progress,
                format!("Node {} - Ready", target.name),
                StatusKind::Done,
            );
            ready.insert(target.name.clone());
        }
        if ready.len() == targets.len() || start.elapsed() >= timeout {
            break;
        }
        operation
            .sleep(Duration::from_secs(READINESS_SLEEP))
            .await?;
    }
    for target in targets {
        if !ready.contains(&target.name) {
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

async fn start_vm(device_name: &str, node_name: &str, state: &AppState) -> NodeActionResult {
    let qemu = state.qemu.clone();
    let device_name = device_name.to_string();
    let node_name_owned = node_name.to_string();

    match tokio::task::spawn_blocking(move || -> Result<NodeActionResult> {
        let node_name = node_name_owned;
        let conn = qemu.connect().context("Failed to connect to libvirt")?;
        let domain = match virt::domain::Domain::lookup_by_name(&conn, &device_name) {
            Ok(d) => d,
            Err(e) => {
                return Ok(NodeActionResult {
                    name: node_name,
                    success: false,
                    message: format!("Domain not found: {}", e),
                });
            }
        };

        match domain.get_state() {
            Ok((state, _reason)) => {
                if state == VIR_DOMAIN_SHUTOFF {
                    // Cold boot a defined-but-inactive domain
                    match domain.create() {
                        Ok(_) => Ok(NodeActionResult {
                            name: node_name,
                            success: true,
                            message: "Started".to_string(),
                        }),
                        Err(e) => Ok(NodeActionResult {
                            name: node_name,
                            success: false,
                            message: format!("Failed to start: {}", e),
                        }),
                    }
                } else if state == VIR_DOMAIN_PAUSED {
                    // Resume a paused domain
                    match domain.resume() {
                        Ok(_) => Ok(NodeActionResult {
                            name: node_name,
                            success: true,
                            message: "Resumed".to_string(),
                        }),
                        Err(e) => Ok(NodeActionResult {
                            name: node_name,
                            success: false,
                            message: format!("Failed to resume: {}", e),
                        }),
                    }
                } else {
                    // Already running or other state
                    Ok(NodeActionResult {
                        name: node_name,
                        success: true,
                        message: "Already running".to_string(),
                    })
                }
            }
            Err(e) => Ok(NodeActionResult {
                name: node_name,
                success: false,
                message: format!("Failed to get state: {}", e),
            }),
        }
    })
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!("Libvirt error: {}", e),
        },
        Err(e) => NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!("Task panicked: {}", e),
        },
    }
}

async fn start_container_node(
    device_name: &str,
    node_name: &str,
    state: &AppState,
) -> NodeActionResult {
    match container::unpause_container(&state.docker, device_name).await {
        Ok(()) => NodeActionResult {
            name: node_name.to_string(),
            success: true,
            message: "Unpaused".to_string(),
        },
        Err(e) => NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!("Failed to unpause: {:#}", e),
        },
    }
}

/// Re-attach eBPF redirect programs on P2p links after VM cold boot.
///
/// When a VM is cold-booted, libvirt creates new tap devices with new ifindexes.
/// The peer side's eBPF program still points to the old (stale) ifindex.
/// This function re-attaches eBPF on both sides of each affected P2p link.
async fn reattach_p2p_ebpf_for_nodes(
    lab_id: &str,
    node_names: &[String],
    lab_record_id: &RecordId,
    state: &AppState,
) -> Result<()> {
    let db_links = db::list_links_by_lab(&state.db, lab_record_id.clone()).await?;
    let db_nodes = db::list_nodes_by_lab(&state.db, lab_record_id.clone()).await?;

    // Build node name -> RecordId lookup
    let node_record_ids: std::collections::HashMap<String, RecordId> = db_nodes
        .iter()
        .filter_map(|n| n.id.clone().map(|id| (n.name.clone(), id)))
        .collect();

    let target_record_ids: Vec<&RecordId> = node_names
        .iter()
        .filter_map(|name| node_record_ids.get(name))
        .collect();

    for link in &db_links {
        if link.kind != data::BridgeKind::P2p {
            continue;
        }

        // Only process links that involve one of the cold-booted nodes
        let involves_target = target_record_ids
            .iter()
            .any(|id| link.node_a == **id || link.node_b == **id);
        if !involves_target {
            continue;
        }

        let ifindex_a = network::get_ifindex(&link.tap_a)
            .await
            .context(format!("failed to get ifindex for {}", link.tap_a))?;
        let ifindex_b = network::get_ifindex(&link.tap_b)
            .await
            .context(format!("failed to get ifindex for {}", link.tap_b))?;

        network::attach_p2p_redirect(&link.tap_a, ifindex_b)
            .context(format!("failed to attach eBPF redirect on {}", link.tap_a))?;
        network::attach_p2p_redirect(&link.tap_b, ifindex_a)
            .context(format!("failed to attach eBPF redirect on {}", link.tap_b))?;

        // Re-apply link impairment if configured
        if link.delay_us > 0 || link.loss_percent > 0.0 {
            let netem = network::LinkImpairment {
                delay_us: link.delay_us,
                jitter_us: link.jitter_us,
                loss_percent: link.loss_percent,
                reorder_percent: 0.0,
                corrupt_percent: 0.0,
            };
            network::apply_netem(ifindex_a as i32, &netem).await?;
            network::apply_netem(ifindex_b as i32, &netem).await?;
        }

        tracing::info!(
            lab_id = %lab_id,
            tap_a = %link.tap_a,
            tap_b = %link.tap_b,
            "Re-attached eBPF P2p redirect after cold boot"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
  { name = "skip", model = "ubuntu_linux", skip_ready_check = true },
  { name = "uk", model = "generic_unikernel", ready_port = 8080 },
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
  { name = "talos-custom", model = "talos_linux", ready_port = 6443 },
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
  { name = "skip", model = "ubuntu_linux", skip_ready_check = true },
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
}
