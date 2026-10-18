use std::net::Ipv4Addr;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use opentelemetry::KeyValue;
use tracing::instrument;

use shared::data::{
    ClusterRole, LabNodeActionResponse, NodeActionResult, NodeKind, NodeModel, NodeState, RecordId,
};
use shared::konst::{VM_SHUTDOWN_POLL, VM_SHUTDOWN_TIMEOUT};

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::talos;

/// Why unikernels are powered off without a shutdown request.
const UNIKERNEL_POWER_OFF: &str = "unikernels have no OS to shut down";
/// Why Talos nodes outside a Sherpa-built cluster are powered off: Talos ignores the ACPI
/// power button, and Sherpa has no credentials for their Talos API.
const TALOS_POWER_OFF: &str = "Talos ignores ACPI and the node is not in a Sherpa-built cluster";

/// How `down` stops a node.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    /// Ask the guest to shut down: the QEMU guest agent if present, otherwise ACPI.
    Acpi,
    /// Ask Talos to shut down through its API, with the cluster's credentials.
    TalosApi,
    /// Power off without asking; the reason is reported.
    PowerOff(&'static str),
    /// Pause a container.
    Pause,
}

/// A node's route, resolved with what it needs.
enum ShutdownMethod {
    Acpi,
    TalosApi {
        cluster: Box<talos::LabCluster>,
        address: Ipv4Addr,
    },
    PowerOff(&'static str),
    /// The clean shutdown could not be prepared; power off and report why.
    Unavailable(String),
    Pause,
}

/// What `stop_domain` does with a running guest.
enum StopPlan {
    /// Send a shutdown request through libvirt, then wait.
    RequestShutdown,
    /// A shutdown was already requested (through the Talos API); wait.
    AwaitShutdown,
    /// Power off without asking; the reason is reported.
    PowerOff(&'static str),
    /// Asking failed; power off and report why.
    RequestFailed(String),
}

/// A domain `down` may need to stop.
enum DomainCheck {
    Running(virt::domain::Domain),
    /// Missing, already stopped or unreadable: the node's result.
    Done(NodeActionResult),
}

/// How a running VM or unikernel was stopped.
#[derive(Debug, PartialEq, Eq)]
enum VmStop {
    /// The guest shut itself down after a shutdown request.
    Graceful,
    /// The guest was still running when the clean shutdown timeout passed.
    ForcedAfterTimeout,
    /// The shutdown request failed, so the guest was powered off.
    ForcedAfterRefusal(String),
    /// Powered off without a request, by design; the reason is reported.
    PoweredOff(&'static str),
}

/// Shutdown all (or a specific) node(s) for a lab.
///
/// VMs: a clean shutdown request (the guest agent if present, otherwise the ACPI power
/// button; for Talos cluster nodes, the Talos API), then up to `VM_SHUTDOWN_TIMEOUT` seconds
/// for the guest to stop, and a forced power-off only if it has not. Talos cluster controllers
/// stop after every other node. Unikernels and Talos nodes outside a Sherpa-built cluster:
/// powered off. Containers: paused.
/// DB node state is updated to `Stopped` once a node has stopped.
///
/// The caller must own the lab or be an admin.
#[instrument(skip(ctx, state), fields(username = %ctx.username, %lab_id))]
pub async fn shutdown_lab_nodes(
    ctx: &AuthContext,
    lab_id: &str,
    node_name: Option<&str>,
    state: &AppState,
) -> Result<LabNodeActionResponse> {
    let start = Instant::now();

    let db_lab = require_lab_access(ctx, &state.db, lab_id).await?;

    let lab_record_id = db_lab
        .id
        .clone()
        .ok_or_else(|| anyhow!("Lab missing record ID"))?;

    // Get all nodes for this lab
    let db_nodes = db::list_nodes_by_lab(&state.db, lab_record_id)
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

    // Talos cluster controllers stop last: workers drain through the Kubernetes API, which
    // runs on the controllers.
    let mut first_wave = Vec::new();
    let mut last_wave = Vec::new();
    let mut immediate_results = Vec::new();

    for node in &target_nodes {
        let device_name = format!("{}-{}", node.name, lab_id);

        let Some(node_image) = node_images
            .iter()
            .find(|img| img.id.as_ref() == Some(&node.image))
        else {
            immediate_results.push(NodeActionResult {
                name: node.name.clone(),
                success: false,
                message: "Node image not found in database".to_string(),
            });
            continue;
        };

        let mut membership = None;
        let mut lookup_error = None;
        if node_image.model == NodeModel::TalosLinux {
            match talos::find_member_cluster(lab_id, &node.name) {
                Ok(found) => membership = found,
                Err(error) => lookup_error = Some(format!("{error:#}")),
            }
        }
        let role = membership.as_ref().map(|(_, role, _)| *role);
        let route = shutdown_route(&node_image.kind, node_image.model, role);
        let method = match (route, membership, lookup_error) {
            (_, _, Some(error)) => ShutdownMethod::Unavailable(format!(
                "could not load the node's Talos cluster: {error}"
            )),
            (Route::TalosApi, Some((cluster, _, address)), None) => ShutdownMethod::TalosApi {
                cluster: Box::new(cluster),
                address,
            },
            (Route::TalosApi, None, None) => {
                ShutdownMethod::Unavailable("no Talos cluster found for the node".to_string())
            }
            (Route::Acpi, _, None) => ShutdownMethod::Acpi,
            (Route::PowerOff(reason), _, None) => ShutdownMethod::PowerOff(reason),
            (Route::Pause, _, None) => ShutdownMethod::Pause,
        };

        let node_name = node.name.clone();
        let node_id = node.id.clone();
        let state = state.clone();
        let future = async move {
            let result = match method {
                ShutdownMethod::Pause => shutdown_container(&device_name, &node_name, &state).await,
                method => shutdown_vm(&device_name, &node_name, method, &state).await,
            };
            (result, node_name, node_id)
        };
        if role == Some(ClusterRole::ControlPlane) {
            last_wave.push(future);
        } else {
            first_wave.push(future);
        }
    }

    // Run each wave's operations concurrently
    let mut concurrent_results = join_all(first_wave).await;
    concurrent_results.extend(join_all(last_wave).await);

    // Collect results and update DB state
    let mut results = immediate_results;
    for (result, node_name, node_id) in concurrent_results {
        if result.success
            && let Some(id) = node_id
            && let Err(e) = db::update_node_state(&state.db, id, NodeState::Stopped).await
        {
            tracing::warn!("Failed to update DB state for node '{}': {}", node_name, e);
        }
        results.push(result);
    }

    if node_name.is_none()
        && let Some(id) = db_lab.tailscale_container_id.as_deref()
    {
        let stopped =
            container::tailscale::stop_gateway(&state.docker, id, lab_id, &state.config.tailscale)
                .await;
        results.push(NodeActionResult {
            name: "Tailscale gateway".into(),
            success: stopped.is_ok(),
            message: match stopped {
                Ok(()) => "Stopped".into(),
                Err(error) => format!("Unable to stop Tailscale gateway: {error:#}"),
            },
        });
    }
    let tailscale = super::tailscale::inspect(state, &db_lab).await;

    state.metrics.operation_duration.record(
        start.elapsed().as_secs_f64(),
        &[KeyValue::new("operation.type", "down")],
    );

    Ok(LabNodeActionResponse { results, tailscale })
}

/// How `down` stops a node, decided from its kind, model and Talos cluster role.
fn shutdown_route(kind: &NodeKind, model: NodeModel, role: Option<ClusterRole>) -> Route {
    match kind {
        NodeKind::Container => Route::Pause,
        NodeKind::Unikernel => Route::PowerOff(UNIKERNEL_POWER_OFF),
        NodeKind::VirtualMachine if model == NodeModel::TalosLinux => match role {
            Some(_) => Route::TalosApi,
            None => Route::PowerOff(TALOS_POWER_OFF),
        },
        NodeKind::VirtualMachine => Route::Acpi,
    }
}

fn stop_message(stop: &VmStop) -> String {
    match stop {
        VmStop::Graceful => "Shut down".to_string(),
        VmStop::ForcedAfterTimeout => format!(
            "Powered off (forced: still running {VM_SHUTDOWN_TIMEOUT}s after a shutdown request)"
        ),
        VmStop::ForcedAfterRefusal(error) => {
            format!("Powered off (forced: shutdown request failed: {error})")
        }
        VmStop::PoweredOff(reason) => format!("Powered off ({reason})"),
    }
}

/// Stop a running domain according to its plan. Waiting for a requested shutdown blocks for up
/// to `VM_SHUTDOWN_TIMEOUT` seconds; run it on a blocking thread.
fn stop_domain(domain: &virt::domain::Domain, plan: StopPlan) -> Result<VmStop> {
    let stop = match plan {
        StopPlan::PowerOff(reason) => VmStop::PoweredOff(reason),
        StopPlan::RequestFailed(error) => VmStop::ForcedAfterRefusal(error),
        StopPlan::RequestShutdown => match domain.shutdown() {
            Ok(_) => wait_for_shutdown(domain)?,
            Err(error) => VmStop::ForcedAfterRefusal(error.to_string()),
        },
        StopPlan::AwaitShutdown => wait_for_shutdown(domain)?,
    };
    // The guest may have stopped on its own between the last check and now.
    if domain.is_active().context("Failed to check state")? {
        domain.destroy().context("Failed to power off")?;
    }
    Ok(stop)
}

/// Poll until a guest that was asked to shut down has stopped, or the timeout passes.
fn wait_for_shutdown(domain: &virt::domain::Domain) -> Result<VmStop> {
    let deadline = Instant::now() + Duration::from_secs(VM_SHUTDOWN_TIMEOUT);
    loop {
        if !domain
            .is_active()
            .context("Failed to check state during shutdown")?
        {
            return Ok(VmStop::Graceful);
        }
        if Instant::now() >= deadline {
            return Ok(VmStop::ForcedAfterTimeout);
        }
        thread::sleep(Duration::from_secs(VM_SHUTDOWN_POLL));
    }
}

/// Look up a domain: running, or a finished node result when there is nothing to stop.
fn check_domain(conn: &virt::connect::Connect, device_name: &str, node_name: &str) -> DomainCheck {
    let domain = match virt::domain::Domain::lookup_by_name(conn, device_name) {
        Ok(domain) => domain,
        Err(e) => {
            return DomainCheck::Done(NodeActionResult {
                name: node_name.to_string(),
                success: false,
                message: format!("Domain not found: {}", e),
            });
        }
    };
    match domain.is_active() {
        Ok(true) => DomainCheck::Running(domain),
        Ok(false) => DomainCheck::Done(NodeActionResult {
            name: node_name.to_string(),
            success: true,
            message: "Already stopped".to_string(),
        }),
        Err(e) => DomainCheck::Done(NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!("Failed to check state: {}", e),
        }),
    }
}

async fn shutdown_vm(
    device_name: &str,
    node_name: &str,
    method: ShutdownMethod,
    state: &AppState,
) -> NodeActionResult {
    // A Talos shutdown is requested over the network first; the domain is then watched.
    let plan = match method {
        ShutdownMethod::Acpi => StopPlan::RequestShutdown,
        ShutdownMethod::TalosApi { cluster, address } => {
            match talos::request_shutdown(&cluster, address).await {
                Ok(()) => StopPlan::AwaitShutdown,
                Err(error) => StopPlan::RequestFailed(format!("{error:#}")),
            }
        }
        ShutdownMethod::PowerOff(reason) => StopPlan::PowerOff(reason),
        ShutdownMethod::Unavailable(reason) => StopPlan::RequestFailed(reason),
        ShutdownMethod::Pause => {
            return NodeActionResult {
                name: node_name.to_string(),
                success: false,
                message: "Containers are paused, not shut down".to_string(),
            };
        }
    };

    let qemu = state.qemu.clone();
    let device_name = device_name.to_string();
    let node_name_owned = node_name.to_string();
    match tokio::task::spawn_blocking(move || -> Result<NodeActionResult> {
        let node_name = node_name_owned;
        let conn = qemu.connect().context("Failed to connect to libvirt")?;
        let domain = match check_domain(&conn, &device_name, &node_name) {
            DomainCheck::Running(domain) => domain,
            DomainCheck::Done(result) => return Ok(result),
        };
        Ok(match stop_domain(&domain, plan) {
            Ok(stop) => NodeActionResult {
                name: node_name,
                success: true,
                message: stop_message(&stop),
            },
            Err(e) => NodeActionResult {
                name: node_name,
                success: false,
                message: format!("{e:#}"),
            },
        })
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

async fn shutdown_container(
    device_name: &str,
    node_name: &str,
    state: &AppState,
) -> NodeActionResult {
    match container::pause_container(&state.docker, device_name).await {
        Ok(()) => NodeActionResult {
            name: node_name.to_string(),
            success: true,
            message: "Paused".to_string(),
        },
        Err(e) => NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!("Failed to pause: {:#}", e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_are_routed_by_kind_model_and_cluster_role() {
        let vm = NodeKind::VirtualMachine;
        assert_eq!(
            shutdown_route(&vm, NodeModel::UbuntuLinux, None),
            Route::Acpi
        );
        assert_eq!(
            shutdown_route(&vm, NodeModel::TalosLinux, Some(ClusterRole::Worker)),
            Route::TalosApi
        );
        assert_eq!(
            shutdown_route(&vm, NodeModel::TalosLinux, Some(ClusterRole::ControlPlane)),
            Route::TalosApi
        );
        assert_eq!(
            shutdown_route(&vm, NodeModel::TalosLinux, None),
            Route::PowerOff(TALOS_POWER_OFF)
        );
        assert_eq!(
            shutdown_route(&NodeKind::Unikernel, NodeModel::NanosUnikernel, None),
            Route::PowerOff(UNIKERNEL_POWER_OFF)
        );
        assert_eq!(
            shutdown_route(&NodeKind::Container, NodeModel::GenericContainer, None),
            Route::Pause
        );
    }

    #[test]
    fn stop_messages_say_whether_the_guest_was_forced_off() {
        assert_eq!(stop_message(&VmStop::Graceful), "Shut down");
        assert!(stop_message(&VmStop::ForcedAfterTimeout).contains("forced: still running 60s"));
        assert!(
            stop_message(&VmStop::ForcedAfterRefusal("no ACPI".to_string()))
                .contains("forced: shutdown request failed: no ACPI")
        );
        // Powering off by design is not reported as forced.
        let designed = stop_message(&VmStop::PoweredOff(TALOS_POWER_OFF));
        assert!(designed.starts_with("Powered off (") && !designed.contains("forced"));
    }
}
