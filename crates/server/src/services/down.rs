use bollard::secret::ContainerStateStatusEnum;
use std::net::Ipv4Addr;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use bollard::Docker;
use container::tailscale::{Logout, stop_gateway};
use futures::future::join_all;
use opentelemetry::KeyValue;
use tokio::task;
use tracing::instrument;
use virt::connect::Connect;
use virt::domain::Domain;

use shared::data::{
    ClusterRole, LabNodeActionResponse, ModelKind, NodeActionResult, NodeConfig, NodeKind,
    NodeModel, OperationEvent, TailscaleGatewaySettings,
};
use shared::konst::SHERPA_LABS_PATH;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::error::Unexpected;
use crate::services::lab_lifecycle;
use crate::services::lab_nodes::load_target_nodes;
use crate::services::lab_operations::{self, Operation};
use crate::services::node_lifecycle;
use crate::services::talos;

/// Seconds `down` waits for a VM to shut down cleanly before powering it off.
const VM_SHUTDOWN_TIMEOUT: u64 = 60;
/// Seconds between VM power state checks while waiting for a clean shutdown.
const VM_SHUTDOWN_POLL: u64 = 1;

/// Why unikernels are powered off without a shutdown request.
const UNIKERNEL_POWER_OFF: &str = "unikernels have no OS to shut down";
/// Why Talos nodes outside a Sherpa-built cluster are powered off: Talos ignores the ACPI
/// power button, and Sherpa has no credentials for their Talos API.
const TALOS_POWER_OFF: &str = "Talos ignores ACPI and the node is not in a Sherpa-built cluster";

/// How `down` stops a node.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    Acpi,
    TalosApi,
    PowerOff(&'static str),
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
    Unavailable(String),
    Pause,
}

/// What `stop_domain` does with a running guest.
enum StopPlan {
    RequestShutdown,
    AwaitShutdown,
    PowerOff(&'static str),
    RequestFailed(String),
}

/// A domain `down` may need to stop.
enum DomainCheck {
    Running(Domain),
    Done(NodeActionResult),
}

/// How a running VM or unikernel was stopped.
#[derive(Debug, PartialEq, Eq)]
enum VmStop {
    Graceful,
    ForcedAfterTimeout,
    ForcedAfterRefusal(String),
    PoweredOff(&'static str),
}

/// Shutdown all (or a specific) node(s) for a lab.
///
/// VMs: a clean shutdown request (the guest agent if present, otherwise the ACPI power
/// button; for Talos cluster nodes, the Talos API), then up to `VM_SHUTDOWN_TIMEOUT` seconds
/// for the guest to stop, and a forced power-off only if it has not. Talos cluster controllers
/// stop after every other node. Unikernels and Talos nodes outside a Sherpa-built cluster:
/// powered off. Containers: paused.
/// DB node state is updated to `Stopped` once a node has stopped. A node an operation is
/// building, or one with no domain or container, is refused before anything is done to it.
///
/// Down holds the lab's operation lease, so a destroy waits for it and the scanner leaves the
/// lab alone while it runs.
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

    require_lab_access(ctx, &state.db, lab_id).await?;
    // Destroying the lab stops this operation; see `lab_operations`.
    let operation = lab_operations::begin(lab_id, Operation::Down).await;
    operation.ensure_active()?;
    // Check again under the lease: a destroy that finished while this waited has removed the lab,
    // and only a lab `up` has built, and no destroy has started on, has nodes to operate on.
    let db_lab = require_lab_access(ctx, &state.db, lab_id).await?;
    lab_lifecycle::require_ready(&db_lab)?;

    let lab_record_id = db_lab
        .id
        .clone()
        .ok_or_else(|| anyhow!("Lab missing record ID"))?;

    // The lab's Talos clusters, for the nodes that shut down through the Talos API.
    let clusters = talos::load_clusters(
        &state.db,
        Path::new(SHERPA_LABS_PATH),
        &lab_record_id,
        lab_id,
    )
    .await
    .map_err(|error| {
        Unexpected::log(&error, "Failed to load the lab's Talos clusters").to_string()
    });
    let (target_nodes, node_images) =
        load_target_nodes(&state.db, lab_record_id, lab_id, node_name).await?;

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
                links: Vec::new(),
            });
            continue;
        };
        if let Err(refusal) = node.state.on_operation(OperationEvent::Stopped) {
            immediate_results.push(NodeActionResult {
                name: node.name.clone(),
                success: false,
                message: format!("Cannot stop: {refusal}"),
                links: Vec::new(),
            });
            continue;
        }

        let (method, role) = shutdown_method(&clusters, &node.name, node_image);

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
    for (mut result, node_name, node_id) in concurrent_results {
        if result.success
            && let Some(id) = node_id
            && let Err(error) =
                node_lifecycle::record(&state.db, &operation, &id, OperationEvent::Stopped).await
        {
            tracing::warn!(node = %node_name, error = %error, "Failed to record the node stopped");
            result.success = false;
            result.message = format!(
                "{}, but not recorded: {}",
                result.message,
                Unexpected::log(&error, "Failed to record a node's state")
            );
        }
        results.push(result);
    }

    if node_name.is_none()
        && let Some(id) = db_lab.tailscale_container_id.as_deref()
    {
        results
            .push(stop_tailscale_gateway(&state.docker, &state.config.tailscale, id, lab_id).await);
    }
    let tailscale = super::tailscale::inspect(state, &db_lab).await;

    state.metrics.operation_duration.record(
        start.elapsed().as_secs_f64(),
        &[KeyValue::new("operation.type", "down")],
    );

    Ok(LabNodeActionResponse { results, tailscale })
}

/// How to stop the node, and its Talos cluster role. A Talos node's cluster is looked up in
/// the lab's `clusters`, or why they could not be loaded; when they could not, or a cluster
/// node has no cluster, the node is powered off with the reason.
fn shutdown_method(
    clusters: &Result<Vec<talos::LabCluster>, String>,
    node_name: &str,
    node_image: &NodeConfig,
) -> (ShutdownMethod, Option<ClusterRole>) {
    let mut membership = None;
    let mut lookup_error = None;
    if node_image.model == NodeModel::TalosLinux {
        match clusters {
            Ok(clusters) => membership = talos::find_member_cluster(clusters, node_name),
            Err(error) => lookup_error = Some(error.clone()),
        }
    }
    let role = membership.as_ref().map(|(_, role, _)| *role);
    let model_kind = match node_image.model_kind() {
        Ok(model_kind) => model_kind,
        Err(error) => return (ShutdownMethod::Unavailable(format!("{error:#}")), role),
    };
    let route = shutdown_route(model_kind, role);
    let method = match (route, membership, lookup_error) {
        (_, _, Some(error)) => {
            ShutdownMethod::Unavailable(format!("could not load the node's Talos cluster: {error}"))
        }
        (Route::TalosApi, Some((cluster, _, address)), None) => ShutdownMethod::TalosApi {
            cluster: Box::new(cluster.clone()),
            address,
        },
        (Route::TalosApi, None, None) => {
            ShutdownMethod::Unavailable("no Talos cluster found for the node".to_string())
        }
        (Route::Acpi, _, None) => ShutdownMethod::Acpi,
        (Route::PowerOff(reason), _, None) => ShutdownMethod::PowerOff(reason),
        (Route::Pause, _, None) => ShutdownMethod::Pause,
    };
    (method, role)
}

/// Log the lab's Tailscale gateway out and stop it, as a node result.
#[instrument(skip(docker, settings), fields(%lab_id), level = "debug")]
async fn stop_tailscale_gateway(
    docker: &Docker,
    settings: &TailscaleGatewaySettings,
    container_id: &str,
    lab_id: &str,
) -> NodeActionResult {
    let stopped = stop_gateway(docker, container_id, lab_id, settings).await;
    let (success, message) = match stopped {
        Ok(Logout::Done) => (true, "Stopped".into()),
        Ok(Logout::Failed(error)) => (
            false,
            format!(
                "Stopped, but not logged out of the tailnet: {}. The device stays on the tailnet until Tailscale removes it (ephemeral key) or you remove it in the Tailscale admin console",
                Unexpected::log(&error, "Unable to log the Tailscale gateway out")
            ),
        ),
        Err(error) => (
            false,
            format!(
                "Unable to stop Tailscale gateway: {}",
                Unexpected::log(&error, "Unable to stop Tailscale gateway")
            ),
        ),
    };
    NodeActionResult {
        name: "Tailscale gateway".into(),
        success,
        message,
        links: Vec::new(),
    }
}

/// How `down` stops a node, decided from its kind, model and Talos cluster role.
fn shutdown_route(model_kind: ModelKind, role: Option<ClusterRole>) -> Route {
    match model_kind.kind() {
        NodeKind::Container if model_kind.runs_in_sandbox() => Route::Acpi,
        NodeKind::Container => Route::Pause,
        NodeKind::Unikernel => Route::PowerOff(UNIKERNEL_POWER_OFF),
        NodeKind::VirtualMachine if model_kind.model() == NodeModel::TalosLinux => match role {
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
fn stop_domain(domain: &Domain, plan: StopPlan) -> Result<VmStop> {
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
fn wait_for_shutdown(domain: &Domain) -> Result<VmStop> {
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
fn check_domain(conn: &Connect, device_name: &str, node_name: &str) -> DomainCheck {
    let domain = match Domain::lookup_by_name(conn, device_name) {
        Ok(domain) => domain,
        Err(e) => {
            return DomainCheck::Done(NodeActionResult {
                name: node_name.to_string(),
                success: false,
                message: format!(
                    "Domain not found: {}",
                    Unexpected::log(&e, "Domain not found")
                ),
                links: Vec::new(),
            });
        }
    };
    match domain.is_active() {
        Ok(true) => DomainCheck::Running(domain),
        Ok(false) => DomainCheck::Done(NodeActionResult {
            name: node_name.to_string(),
            success: true,
            message: "Already stopped".to_string(),
            links: Vec::new(),
        }),
        Err(e) => DomainCheck::Done(NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!(
                "Failed to check state: {}",
                Unexpected::log(&e, "Failed to check state")
            ),
            links: Vec::new(),
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
                Err(error) => StopPlan::RequestFailed(
                    Unexpected::log(&error, "Failed to stop a node").to_string(),
                ),
            }
        }
        ShutdownMethod::PowerOff(reason) => StopPlan::PowerOff(reason),
        ShutdownMethod::Unavailable(reason) => StopPlan::RequestFailed(reason),
        ShutdownMethod::Pause => {
            return NodeActionResult {
                name: node_name.to_string(),
                success: false,
                message: "Containers are paused, not shut down".to_string(),
                links: Vec::new(),
            };
        }
    };

    let qemu = state.qemu.clone();
    let device_name = device_name.to_string();
    let node_name_owned = node_name.to_string();
    match task::spawn_blocking(move || -> Result<NodeActionResult> {
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
                links: Vec::new(),
            },
            Err(e) => NodeActionResult {
                name: node_name,
                success: false,
                message: Unexpected::log(&e, "Failed to stop a node").to_string(),
                links: Vec::new(),
            },
        })
    })
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!("Libvirt error: {}", Unexpected::log(&e, "Libvirt error")),
            links: Vec::new(),
        },
        Err(e) => NodeActionResult {
            name: node_name.to_string(),
            success: false,
            message: format!("Task panicked: {}", Unexpected::log(&e, "Task panicked")),
            links: Vec::new(),
        },
    }
}

/// Whether `down` pauses a container in `status`, or it is already down.
fn container_down_step(status: Option<ContainerStateStatusEnum>) -> Result<(), &'static str> {
    match status {
        Some(ContainerStateStatusEnum::RUNNING) => Ok(()),
        Some(ContainerStateStatusEnum::PAUSED) => Err("Already paused"),
        _ => Err("Not running"),
    }
}

async fn shutdown_container(
    device_name: &str,
    node_name: &str,
    state: &AppState,
) -> NodeActionResult {
    let result = |success: bool, message: String| NodeActionResult {
        name: node_name.to_string(),
        success,
        message,
        links: Vec::new(),
    };
    // Pausing a container that is not running fails, so a second down would report errors.
    let status = match container::container_status(&state.docker, device_name).await {
        Ok(status) => status,
        Err(e) => {
            return result(
                false,
                format!(
                    "Failed to check the container: {}",
                    Unexpected::log(&e, "Failed to check the container")
                ),
            );
        }
    };
    if let Err(already) = container_down_step(status) {
        return result(true, already.to_string());
    }
    match container::pause_container(&state.docker, device_name).await {
        Ok(()) => result(true, "Paused".to_string()),
        Err(e) => result(
            false,
            format!(
                "Failed to pause: {}",
                Unexpected::log(&e, "Failed to pause")
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::services::docker_mock::{DockerMock, Fault};
    use crate::services::talos::lab::tests::cluster_plans;
    use crate::services::talos::secrets::generate_cluster_secrets;

    use super::*;
    use shared::data::ModelKind;

    /// The clusters of `cluster_plans`, as `load_clusters` returns them.
    fn clusters() -> Vec<talos::LabCluster> {
        cluster_plans()
            .into_iter()
            .map(|plan| talos::LabCluster {
                plan,
                secrets: generate_cluster_secrets().unwrap(),
            })
            .collect()
    }

    #[test]
    fn cluster_shutdown_003_members_shut_down_through_the_talos_api_of_their_cluster() {
        let talos = ModelKind::default_for(NodeModel::TalosLinux).node_config();
        let clusters = Ok(clusters());

        for (node, expected_role, expected_address) in [
            ("w1", ClusterRole::Worker, Ipv4Addr::new(172, 31, 1, 24)),
            (
                "cp1",
                ClusterRole::ControlPlane,
                Ipv4Addr::new(172, 31, 1, 21),
            ),
        ] {
            let (method, role) = shutdown_method(&clusters, node, &talos);
            let ShutdownMethod::TalosApi { cluster, address } = method else {
                panic!("{node} does not shut down through the Talos API");
            };
            assert_eq!(cluster.plan.name, "ha", "{node}");
            assert_eq!(address, expected_address, "{node}");
            assert_eq!(role, Some(expected_role), "{node}");
            assert_eq!(
                shutdown_route(ModelKind::default_for(NodeModel::TalosLinux), role),
                Route::TalosApi,
                "{node}"
            );
        }
    }

    #[test]
    fn talos_node_002_a_talos_node_in_no_cluster_is_powered_off_with_the_reason() {
        let talos = ModelKind::default_for(NodeModel::TalosLinux).node_config();
        let clusters = Ok(clusters());

        let (method, role) = shutdown_method(&clusters, "loose", &talos);
        let ShutdownMethod::PowerOff(reason) = method else {
            panic!("a Talos node in no cluster is not powered off");
        };
        assert!(reason.contains("not in a Sherpa-built cluster"), "{reason}");
        assert_eq!(role, None);
        assert_eq!(
            shutdown_route(ModelKind::default_for(NodeModel::TalosLinux), None),
            Route::PowerOff(reason)
        );
    }

    #[test]
    fn cluster_shutdown_004_talos_nodes_power_off_when_the_clusters_cannot_be_loaded() {
        let talos = ModelKind::default_for(NodeModel::TalosLinux).node_config();
        let ubuntu = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        let clusters = Err("secrets are missing".to_owned());

        let (method, _) = shutdown_method(&clusters, "w1", &talos);
        assert!(
            matches!(method, ShutdownMethod::Unavailable(reason) if reason.contains("secrets are missing"))
        );
        let (method, _) = shutdown_method(&clusters, "vm1", &ubuntu);
        assert!(matches!(method, ShutdownMethod::Acpi));
    }

    #[test]
    fn node_down_003_down_pauses_only_a_running_container() {
        assert_eq!(
            container_down_step(Some(ContainerStateStatusEnum::RUNNING)),
            Ok(())
        );
        assert_eq!(
            container_down_step(Some(ContainerStateStatusEnum::PAUSED)),
            Err("Already paused")
        );
        for status in [
            Some(ContainerStateStatusEnum::EXITED),
            Some(ContainerStateStatusEnum::CREATED),
            None,
        ] {
            assert_eq!(container_down_step(status), Err("Not running"));
        }
    }

    #[test]
    fn node_down_001_down_shuts_a_vm_down_powers_a_unikernel_off_and_pauses_a_container() {
        assert_eq!(
            shutdown_route(ModelKind::default_for(NodeModel::UbuntuLinux), None),
            Route::Acpi
        );
        assert_eq!(
            shutdown_route(ModelKind::default_for(NodeModel::NanosUnikernel), None),
            Route::PowerOff(UNIKERNEL_POWER_OFF)
        );
        assert_eq!(
            shutdown_route(
                ModelKind::new(NodeModel::Generic, Some(NodeKind::Container)).unwrap(),
                None
            ),
            Route::Pause
        );
        let ceos = ModelKind::new(NodeModel::AristaEos, Some(NodeKind::Container)).unwrap();
        assert_eq!(shutdown_route(ceos, None), Route::Acpi);
    }

    #[test]
    fn node_down_002_a_vms_result_says_whether_it_shut_down_or_was_forced_off() {
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

    #[tokio::test]
    async fn tailnet_suspend_016_down_reports_a_gateway_whose_logout_fails_as_failed_and_stops_it()
    {
        let mock = DockerMock::new(json!([]), Some(Fault::Logout)).await;
        let result = stop_tailscale_gateway(
            &mock.docker,
            &TailscaleGatewaySettings::default(),
            "gateway-id",
            "test1234",
        )
        .await;
        assert!(!result.success);
        assert!(
            result.message.contains("not logged out"),
            "{}",
            result.message
        );
        let requests = mock.requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .any(|r| r.contains("/containers/gateway-id/exec"))
        );
        assert!(
            requests
                .iter()
                .any(|r| r.contains("/containers/gateway-id/stop"))
        );
    }
}
