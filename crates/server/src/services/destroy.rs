use std::fs;
use std::future::Future;
use std::path::Path;

use anyhow::{Context, Result};
use opentelemetry::KeyValue;
use virt::sys::VIR_DOMAIN_UNDEFINE_NVRAM;

use container::tailscale::{gateway_name, is_gateway, remove_gateway};
use container::{
    Docker, delete_network, kill_container, list_containers, list_networks, remove_container,
};
use libvirt::{delete_disk, list_lab_volumes};
use network::{delete_interface, list_interface_names};
use shared::data::{
    DestroyError, DestroyResponse, DestroySummary, LabEvent, StatusKind, TailscaleGatewaySettings,
};
use shared::konst::{
    BRIDGE_MEMBER_HOST_VETH_PREFIX, BRIDGE_PREFIX, CONTAINER_HOST_VETH_PREFIX, SHERPA_LABS_PATH,
    SHERPA_SECRETS_DIR, SHERPA_STORAGE_POOL, TAP_PREFIX,
};
use shared::util::dir_exists;
use shared::util::is_lab_resource;

use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::lab_lifecycle;
use crate::services::lab_operations::{self, LabHold, StopReason};
use crate::services::node_secrets::remove_lab_secrets;
use crate::services::progress::ProgressSender;

/// Destroy a lab and all its resources
///
/// This function destroys:
/// - Containers (via Docker)
/// - Virtual machines and their disks (via libvirt)
/// - Docker networks
/// - Libvirt networks
/// - Network interfaces (bridges, veths)
/// - Database records
/// - Lab directory
///
/// Error handling: Continue with all resources even if some fail,
/// tracking successes and failures separately. Retain ownership and saved
/// configuration if Tailscale cleanup fails so the owner can retry destruction.
///
/// The caller must own the lab or be an admin. Authorization is checked before any
/// progress is sent or any resource is touched.
#[instrument(skip(ctx, state, progress), fields(username = %ctx.username, %lab_id))]
pub async fn destroy_lab(
    ctx: &AuthContext,
    lab_id: &str,
    state: &AppState,
    progress: ProgressSender,
) -> Result<DestroyResponse> {
    let start_time = std::time::Instant::now();

    let lab = require_lab_access(ctx, &state.db, lab_id).await?;

    tracing::info!(
        lab_id = %lab_id,
        username = %ctx.username,
        "Starting lab destruction"
    );

    // Stop any operation still building or changing the lab, so nothing writes to it after
    // its resources are removed. Hold the lease until destruction finishes.
    let _ = progress.send_status(
        "Stopping operations in progress on this lab...".to_string(),
        StatusKind::Progress,
    );
    let (was_running, stopped) = lab_operations::stop(lab_id, StopReason::Destroy).await;
    if was_running {
        tracing::info!(lab_id = %lab_id, "Stopped operations in progress before destruction");
        let _ = progress.send_status(
            "Stopped an operation in progress".to_string(),
            StatusKind::Done,
        );
    }

    // Read the lab again under the lease: a destroy that held it before this one may have
    // removed it, which is what this destroy was asked to do.
    let lab_name = lab.name.clone();
    if db::find_lab(&state.db, lab_id).await?.is_none() {
        tracing::info!(lab_id = %lab_id, "Lab already destroyed");
        let _ = progress.send_status("Lab already destroyed".to_string(), StatusKind::Done);
        return Ok(DestroyResponse {
            success: true,
            lab_id: lab_id.to_string(),
            lab_name,
            summary: DestroySummary {
                database_records_deleted: true,
                lab_directory_deleted: true,
                ..DestroySummary::default()
            },
            errors: Vec::new(),
        });
    }

    let mut summary = DestroySummary::default();
    let mut errors = Vec::new();
    tear_down_lab(state, &stopped, &mut summary, &mut errors, Some(&progress)).await;

    // Determine overall success
    let success = errors.is_empty();
    let total_duration = start_time.elapsed().as_secs();

    let op_attrs = &[KeyValue::new("operation.type", "destroy")];
    state
        .metrics
        .operation_duration
        .record(start_time.elapsed().as_secs_f64(), op_attrs);
    if !success {
        state.metrics.error_count.add(1, op_attrs);
    }

    tracing::info!(
        lab_id = %lab_id,
        lab_name = %lab_name,
        success = success,
        total_duration_secs = total_duration,
        containers_destroyed = summary.containers_destroyed.len(),
        vms_destroyed = summary.vms_destroyed.len(),
        disks_deleted = summary.disks_deleted.len(),
        docker_networks_destroyed = summary.docker_networks_destroyed.len(),
        libvirt_networks_destroyed = summary.libvirt_networks_destroyed.len(),
        interfaces_deleted = summary.interfaces_deleted.len(),
        total_errors = errors.len(),
        "Lab destruction completed"
    );

    Ok(DestroyResponse {
        success,
        lab_id: lab_id.to_string(),
        lab_name,
        summary,
        errors,
    })
}

/// Remove the lab `hold` is on: move its record to `Destroying`, remove its resources and its
/// folder, and then its record. When anything is left behind, the record stays, `DestroyFailed`,
/// so the lab can be destroyed again. A lab with no record, which only an admin clean reaches,
/// has its resources and folder removed.
#[instrument(skip(state, hold, summary, errors, progress), fields(lab_id = %hold.lab_id()))]
pub(crate) async fn tear_down_lab(
    state: &AppState,
    hold: &impl LabHold,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
    progress: Option<&ProgressSender>,
) {
    let lab_id = hold.lab_id();
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let recorded = match db::find_lab(&state.db, lab_id).await {
        Ok(None) => false,
        Ok(Some(_)) => {
            if let Err(error) =
                lab_lifecycle::record(&state.db, hold, LabEvent::DestroyStarted).await
            {
                errors.push(DestroyError::new("database", lab_id, format!("{error:#}")));
            }
            true
        }
        Err(error) => {
            errors.push(DestroyError::new("database", lab_id, format!("{error:#}")));
            true
        }
    };

    destroy_lab_resources(lab_id, state, summary, errors, progress).await;
    // Its links are gone with its interfaces.
    state.link_states.forget_lab(lab_id);

    let remove_record = async {
        if recorded {
            lab_lifecycle::record(&state.db, hold, LabEvent::Destroyed).await?;
        }
        Ok(())
    };
    cleanup_lab_metadata(lab_id, &lab_dir, remove_record, summary, errors, progress).await;

    if recorded
        && !errors.is_empty()
        && let Err(error) = lab_lifecycle::record(&state.db, hold, LabEvent::DestroyFailed).await
    {
        tracing::error!(lab_id = %lab_id, error = ?error, "Failed to record that the destroy failed");
    }
}

/// Remove the lab's infrastructure: containers and their secret files, VMs and disks, Docker
/// and libvirt networks, and host interfaces. A resource that cannot be removed is recorded in
/// `errors` and the rest still go. With `progress`, each step and what it removed is reported.
#[instrument(skip(state, summary, errors, progress), fields(%lab_id), level = "debug")]
pub(crate) async fn destroy_lab_resources(
    lab_id: &str,
    state: &AppState,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
    progress: Option<&ProgressSender>,
) {
    destroy_compute(lab_id, state, summary, errors, progress).await;
    destroy_networking(lab_id, state, summary, errors, progress).await;
}

/// Teardown steps 1 and 2: containers and their secret files, then VMs and their disks.
async fn destroy_compute(
    lab_id: &str,
    state: &AppState,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
    progress: Option<&ProgressSender>,
) {
    // 1. Destroy containers
    let containers_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Destroying containers");
    send_progress(progress, "Destroying containers...", StatusKind::Progress);
    destroy_containers(
        lab_id,
        &state.docker,
        &state.config.tailscale,
        summary,
        errors,
    )
    .await;
    destroy_lab_secrets(lab_id, Path::new(SHERPA_SECRETS_DIR), errors);
    report_step(
        progress,
        summary.containers_destroyed.is_empty() && summary.containers_failed.is_empty(),
        "No containers to destroy",
        &[("Destroyed container", &summary.containers_destroyed)],
    );
    tracing::info!(
        lab_id = %lab_id,
        destroyed = summary.containers_destroyed.len(),
        failed = summary.containers_failed.len(),
        duration_secs = containers_timer.elapsed().as_secs(),
        "Container destruction completed"
    );

    // 2. Destroy VMs and disks
    let vms_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Destroying VMs and disks");
    send_progress(
        progress,
        "Destroying VMs and disks...",
        StatusKind::Progress,
    );
    destroy_vms_and_disks(lab_id, &state.qemu, summary, errors);
    report_step(
        progress,
        summary.vms_destroyed.is_empty() && summary.vms_failed.is_empty(),
        "No VMs to destroy",
        &[
            ("Destroyed VM", &summary.vms_destroyed),
            ("Deleted disk", &summary.disks_deleted),
        ],
    );
    tracing::info!(
        lab_id = %lab_id,
        vms_destroyed = summary.vms_destroyed.len(),
        vms_failed = summary.vms_failed.len(),
        disks_deleted = summary.disks_deleted.len(),
        disks_failed = summary.disks_failed.len(),
        duration_secs = vms_timer.elapsed().as_secs(),
        "VM and disk destruction completed"
    );
}

/// Teardown steps 3 to 5: Docker and libvirt networks, then host interfaces.
async fn destroy_networking(
    lab_id: &str,
    state: &AppState,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
    progress: Option<&ProgressSender>,
) {
    // 3. Destroy Docker networks
    let docker_net_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Destroying Docker networks");
    send_progress(
        progress,
        "Destroying Docker networks...",
        StatusKind::Progress,
    );
    destroy_docker_networks(lab_id, &state.docker, summary, errors).await;
    report_step(
        progress,
        summary.docker_networks_destroyed.is_empty() && summary.docker_networks_failed.is_empty(),
        "No Docker networks to destroy",
        &[(
            "Destroyed Docker network",
            &summary.docker_networks_destroyed,
        )],
    );
    tracing::info!(
        lab_id = %lab_id,
        destroyed = summary.docker_networks_destroyed.len(),
        failed = summary.docker_networks_failed.len(),
        duration_secs = docker_net_timer.elapsed().as_secs(),
        "Docker network destruction completed"
    );

    // 4. Destroy libvirt networks
    let libvirt_net_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Destroying libvirt networks");
    send_progress(
        progress,
        "Destroying libvirt networks...",
        StatusKind::Progress,
    );
    destroy_libvirt_networks(lab_id, &state.qemu, summary, errors);
    report_step(
        progress,
        summary.libvirt_networks_destroyed.is_empty() && summary.libvirt_networks_failed.is_empty(),
        "No libvirt networks to destroy",
        &[(
            "Destroyed libvirt network",
            &summary.libvirt_networks_destroyed,
        )],
    );
    tracing::info!(
        lab_id = %lab_id,
        destroyed = summary.libvirt_networks_destroyed.len(),
        failed = summary.libvirt_networks_failed.len(),
        duration_secs = libvirt_net_timer.elapsed().as_secs(),
        "Libvirt network destruction completed"
    );

    // 5. Delete network interfaces
    let interfaces_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Deleting network interfaces");
    send_progress(
        progress,
        "Deleting network interfaces...",
        StatusKind::Progress,
    );
    destroy_interfaces(lab_id, summary, errors).await;
    report_step(
        progress,
        summary.interfaces_deleted.is_empty() && summary.interfaces_failed.is_empty(),
        "No network interfaces to delete",
        &[("Deleted interface", &summary.interfaces_deleted)],
    );
    tracing::info!(
        lab_id = %lab_id,
        deleted = summary.interfaces_deleted.len(),
        failed = summary.interfaces_failed.len(),
        duration_secs = interfaces_timer.elapsed().as_secs(),
        "Network interface deletion completed"
    );
}

/// Send a status message when the caller streams progress.
fn send_progress(progress: Option<&ProgressSender>, message: &str, kind: StatusKind) {
    if let Some(progress) = progress {
        let _ = progress.send_status(message.to_owned(), kind);
    }
}

/// Report one teardown step: `none_message` when it found nothing, otherwise each resource it
/// removed, as `label: name`.
fn report_step(
    progress: Option<&ProgressSender>,
    found_nothing: bool,
    none_message: &str,
    removed: &[(&str, &Vec<String>)],
) {
    if found_nothing {
        send_progress(progress, none_message, StatusKind::Info);
        return;
    }
    for (label, names) in removed {
        for name in *names {
            send_progress(progress, &format!("{label}: {name}"), StatusKind::Done);
        }
    }
}

/// Finish filesystem and database cleanup after infrastructure teardown.
///
/// Keep everything when any resource failed to go, so the lab can be destroyed again. Otherwise
/// remove the lab folder, then the record: the record goes last, so a folder that cannot be
/// removed keeps the record its owner destroys again. The database future is only polled once
/// the folder is gone.
#[instrument(skip(database_cleanup, summary, errors, progress), fields(%lab_id), level = "debug")]
pub(crate) async fn cleanup_lab_metadata(
    lab_id: &str,
    lab_dir: &str,
    database_cleanup: impl Future<Output = Result<()>>,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
    progress: Option<&ProgressSender>,
) {
    let send_status = |message: String, kind: StatusKind| {
        if let Some(progress) = progress {
            let _ = progress.send_status(message, kind);
        }
    };
    // Something the lab owns is still there, so keep the record that lets its owner retry.
    if !errors.is_empty() {
        tracing::warn!(lab_id = %lab_id, "Retaining lab ownership and saved configuration after resource cleanup failure");
        send_status(
            "Resource cleanup failed; lab ownership and saved configuration retained for a cleanup retry."
                .to_string(),
            StatusKind::Info,
        );
        return;
    }

    tracing::info!(lab_id = %lab_id, lab_dir = %lab_dir, "Deleting lab directory");
    send_status(
        "Deleting lab directory...".to_string(),
        StatusKind::Progress,
    );
    if dir_exists(lab_dir) {
        if let Err(e) = fs::remove_dir_all(lab_dir) {
            errors.push(DestroyError::new("filesystem", lab_dir, format!("{e:?}")));
            tracing::error!(lab_id = %lab_id, lab_dir = %lab_dir, error = ?e, "Failed to delete lab directory; keeping the lab record");
            return;
        }
        tracing::info!(lab_id = %lab_id, lab_dir = %lab_dir, "Lab directory deleted");
    } else {
        tracing::debug!(lab_id = %lab_id, lab_dir = %lab_dir, "Lab directory already removed");
    }
    summary.lab_directory_deleted = true;
    send_status("Lab directory deleted".to_string(), StatusKind::Done);

    tracing::info!(lab_id = %lab_id, "Cleaning up database records");
    send_status(
        "Cleaning up database records...".to_string(),
        StatusKind::Progress,
    );
    match database_cleanup.await {
        Ok(()) => {
            summary.database_records_deleted = true;
            send_status("Database records cleaned".to_string(), StatusKind::Done);
            tracing::info!(lab_id = %lab_id, "Database cleanup successful");
        }
        Err(e) => {
            errors.push(DestroyError::new("database", lab_id, format!("{e:#}")));
            tracing::error!(lab_id = %lab_id, error = ?e, "Database cleanup failed");
        }
    }
}

/// Remove a lab's container secret files from `root`, recording a failure as a destroy error.
#[instrument(skip(errors), level = "debug")]
pub(crate) fn destroy_lab_secrets(lab_id: &str, root: &Path, errors: &mut Vec<DestroyError>) {
    if let Err(error) = remove_lab_secrets(root, lab_id) {
        tracing::error!(lab_id = %lab_id, error = %format!("{error:#}"), "Failed to remove lab secret files");
        errors.push(DestroyError::new("secrets", lab_id, format!("{error:#}")));
    }
}

/// The name of a container in the lab `lab_id`, given its Docker names, or `None` when it is
/// another lab's or not Sherpa's. Docker names start with `/`.
fn lab_container_name<'a>(names: &'a [String], lab_id: &str) -> Option<&'a str> {
    names
        .iter()
        .map(|name| name.trim_start_matches('/'))
        .find(|name| is_lab_resource(name, lab_id))
}

/// Whether `interface` is one of the lab's host interfaces that Sherpa deletes itself: the
/// bridges, taps and veths it creates outside libvirt and Docker. Only one end of a veth needs
/// deleting.
fn is_lab_interface(interface: &str, lab_id: &str) -> bool {
    let sherpa_made = [
        format!("{BRIDGE_PREFIX}i"),
        format!("{BRIDGE_PREFIX}s"),
        format!("{TAP_PREFIX}a"),
        format!("{TAP_PREFIX}b"),
        BRIDGE_MEMBER_HOST_VETH_PREFIX.to_owned(),
        CONTAINER_HOST_VETH_PREFIX.to_owned(),
    ];
    sherpa_made
        .iter()
        .any(|prefix| interface.starts_with(prefix.as_str()))
        && is_lab_resource(interface, lab_id)
}

/// Destroy all containers for a lab
#[instrument(skip(docker, settings, summary, errors), fields(%lab_id), level = "debug")]
pub(crate) async fn destroy_containers(
    lab_id: &str,
    docker: &Docker,
    settings: &TailscaleGatewaySettings,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
) {
    match list_containers(docker).await {
        Ok(containers) => {
            let lab_containers: Vec<_> = containers
                .iter()
                .filter(|c| {
                    c.names
                        .as_deref()
                        .and_then(|names| lab_container_name(names, lab_id))
                        .is_some()
                })
                .collect();

            tracing::debug!(
                lab_id = %lab_id,
                container_count = lab_containers.len(),
                "Found containers to destroy"
            );

            for container in lab_containers {
                let Some(name) = container
                    .names
                    .as_deref()
                    .and_then(|names| lab_container_name(names, lab_id))
                else {
                    continue;
                };
                // Gateways are removed below after verifying exact ownership.
                if container.labels.as_ref().is_some_and(is_gateway) {
                    continue;
                }
                tracing::debug!(
                    lab_id = %lab_id,
                    container_name = %name,
                    "Destroying container"
                );
                // Best-effort kill — container may not be running
                // (e.g. created but never started during a partial failure).
                if let Err(e) = kill_container(docker, name).await {
                    tracing::debug!(
                        lab_id = %lab_id,
                        container_name = %name,
                        error = ?e,
                        "Kill container failed (may not be running), proceeding to remove"
                    );
                }

                match remove_container(docker, name).await {
                    Ok(()) => {
                        summary.containers_destroyed.push(name.to_string());
                        tracing::info!(
                            lab_id = %lab_id,
                            container_name = %name,
                            "Container destroyed"
                        );
                    }
                    Err(e) => {
                        summary.containers_failed.push(name.to_string());
                        errors.push(DestroyError::new("container", name, format!("{e:?}")));
                        tracing::error!(
                            lab_id = %lab_id,
                            container_name = %name,
                            error = ?e,
                            "Failed to remove container"
                        );
                    }
                }
            }
        }
        Err(e) => {
            errors.push(DestroyError::new(
                "container",
                "list_containers",
                format!("Failed to list containers: {e:?}"),
            ));
            tracing::error!(lab_id = %lab_id, error = ?e, "Failed to list containers");
        }
    }

    // Remove ordinary nodes first, including nodes named like the gateway.
    // Gateway cleanup still runs if listing or removing ordinary nodes fails.
    if let Err(error) = remove_gateway(docker, lab_id, settings).await {
        errors.push(DestroyError::new(
            "tailscale",
            lab_id,
            format!(
                "{error:#}; lab ownership and saved configuration retained for a cleanup retry"
            ),
        ));
        summary.containers_failed.push(gateway_name(lab_id));
    }
}

/// Destroy all VMs and their disks for a lab. A failure is recorded in `errors` and the rest of
/// destroy carries on; a `vm` or `disk` error keeps the lab for a retry.
pub(crate) fn destroy_vms_and_disks(
    lab_id: &str,
    qemu: &libvirt::Qemu,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
) {
    let qemu_conn = match qemu.connect() {
        Ok(conn) => conn,
        Err(e) => {
            errors.push(DestroyError::new(
                "vm",
                "libvirt",
                format!("Failed to connect to libvirt: {e:?}"),
            ));
            tracing::error!(lab_id = %lab_id, error = ?e, "Failed to connect to libvirt to destroy VMs");
            return;
        }
    };

    // Destroy VMs
    let domains = match qemu_conn.list_all_domains(0) {
        Ok(domains) => domains,
        Err(e) => {
            errors.push(DestroyError::new(
                "vm",
                "list_domains",
                format!("Failed to list domains: {e:?}"),
            ));
            tracing::error!(lab_id = %lab_id, error = ?e, "Failed to list libvirt domains");
            return;
        }
    };

    for domain in domains {
        let vm_name = match domain.get_name() {
            Ok(name) => name,
            Err(e) => {
                errors.push(DestroyError::new(
                    "vm",
                    "unknown",
                    format!("Failed to get domain name: {e:?}"),
                ));
                continue;
            }
        };

        if !is_lab_resource(&vm_name, lab_id) {
            continue;
        }

        // Stop before undefining so a failed stop remains discoverable on retry.
        let result = (|| -> Result<()> {
            if domain.is_active().context("Cannot determine VM state")? {
                domain.destroy().context("Failed to destroy domain")?;
            }
            domain
                .undefine_flags(VIR_DOMAIN_UNDEFINE_NVRAM)
                .context("Failed to undefine domain")?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                summary.vms_destroyed.push(vm_name.clone());
                tracing::info!("Destroyed VM: {}", vm_name);
            }
            Err(e) => {
                summary.vms_failed.push(vm_name.clone());
                errors.push(DestroyError::new("vm", &vm_name, format!("{e:?}")));
                tracing::error!("Failed to destroy VM {}: {:?}", vm_name, e);
            }
        }
    }

    if errors.iter().any(|error| error.resource_type == "vm") {
        return;
    }

    // Delete all disks belonging to this lab
    let lab_disks = match list_lab_volumes(&qemu_conn, lab_id) {
        Ok(disks) => disks,
        Err(e) => {
            errors.push(DestroyError::new(
                "disk",
                SHERPA_STORAGE_POOL,
                format!("{e:?}"),
            ));
            tracing::error!(lab_id = %lab_id, error = ?e, "Failed to list lab disks");
            return;
        }
    };

    for disk in &lab_disks {
        match delete_disk(&qemu_conn, disk) {
            Ok(()) => {
                summary.disks_deleted.push(disk.clone());
                tracing::info!("Deleted disk: {}", disk);
            }
            Err(e) => {
                summary.disks_failed.push(disk.clone());
                errors.push(DestroyError::new("disk", disk, format!("{e:?}")));
                tracing::error!("Failed to delete disk {}: {:?}", disk, e);
            }
        }
    }
}

/// Destroy all Docker networks for a lab
pub(crate) async fn destroy_docker_networks(
    lab_id: &str,
    docker: &bollard::Docker,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
) {
    match list_networks(docker).await {
        Ok(container_networks) => {
            for network in container_networks {
                if let Some(network_name) = network.name
                    && is_lab_resource(&network_name, lab_id)
                {
                    match delete_network(docker, &network_name).await {
                        Ok(()) => {
                            summary.docker_networks_destroyed.push(network_name.clone());
                            tracing::info!("Destroyed Docker network: {}", network_name);
                        }
                        Err(e) => {
                            summary.docker_networks_failed.push(network_name.clone());
                            errors.push(DestroyError::new(
                                "docker_network",
                                &network_name,
                                format!("{e:?}"),
                            ));
                            tracing::error!(
                                "Failed to destroy Docker network {}: {:?}",
                                network_name,
                                e
                            );
                        }
                    }
                }
            }
        }
        Err(e) => {
            errors.push(DestroyError::new(
                "docker_network",
                "list_networks",
                format!("Failed to list Docker networks: {e:?}"),
            ));
            tracing::error!("Failed to list Docker networks: {:?}", e);
        }
    }
}

/// Destroy all libvirt networks for a lab. A failure is recorded in `errors` and the rest of
/// destroy carries on.
pub(crate) fn destroy_libvirt_networks(
    lab_id: &str,
    qemu: &libvirt::Qemu,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
) {
    let qemu_conn = match qemu.connect() {
        Ok(conn) => conn,
        Err(e) => {
            errors.push(DestroyError::new(
                "libvirt_network",
                "libvirt",
                format!("Failed to connect to libvirt: {e:?}"),
            ));
            tracing::error!(lab_id = %lab_id, error = ?e, "Failed to connect to libvirt to destroy networks");
            return;
        }
    };
    let networks = match qemu_conn.list_all_networks(0) {
        Ok(networks) => networks,
        Err(e) => {
            errors.push(DestroyError::new(
                "libvirt_network",
                "list_networks",
                format!("Failed to list networks: {e:?}"),
            ));
            tracing::error!(lab_id = %lab_id, error = ?e, "Failed to list libvirt networks");
            return;
        }
    };

    for network in networks {
        let network_name = match network.get_name() {
            Ok(name) => name,
            Err(e) => {
                errors.push(DestroyError::new(
                    "libvirt_network",
                    "unknown",
                    format!("Failed to get network name: {e:?}"),
                ));
                continue;
            }
        };

        if is_lab_resource(&network_name, lab_id) {
            match (|| -> Result<()> {
                let is_active = network.is_active().unwrap_or(false);
                if is_active {
                    network.destroy().context("Failed to destroy network")?;
                }
                network.undefine().context("Failed to undefine network")?;
                Ok(())
            })() {
                Ok(()) => {
                    summary
                        .libvirt_networks_destroyed
                        .push(network_name.clone());
                    tracing::info!("Destroyed libvirt network: {}", network_name);
                }
                Err(e) => {
                    summary.libvirt_networks_failed.push(network_name.clone());
                    errors.push(DestroyError::new(
                        "libvirt_network",
                        &network_name,
                        format!("{e:?}"),
                    ));
                    tracing::error!(
                        "Failed to destroy libvirt network {}: {:?}",
                        network_name,
                        e
                    );
                }
            }
        }
    }
}

/// Delete network interfaces for a lab
pub(crate) async fn destroy_interfaces(
    lab_id: &str,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
) {
    match list_interface_names().await {
        Ok(interfaces) => {
            for interface in interfaces {
                if is_lab_interface(&interface, lab_id) {
                    match delete_interface(&interface).await {
                        Ok(()) => {
                            summary.interfaces_deleted.push(interface.clone());
                            tracing::info!("Deleted interface: {}", interface);
                        }
                        Err(e) => {
                            summary.interfaces_failed.push(interface.clone());
                            errors.push(DestroyError::new(
                                "interface",
                                &interface,
                                format!("{e:?}"),
                            ));
                            tracing::error!("Failed to delete interface {}: {:?}", interface, e);
                        }
                    }
                }
            }
        }
        Err(e) => {
            errors.push(DestroyError::new(
                "interface",
                "find_interfaces",
                format!("Failed to find interfaces: {e:?}"),
            ));
            tracing::error!("Failed to find interfaces: {:?}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use axum::http::{Method, StatusCode, Uri};
    use axum::{Json, Router};
    use bollard::Docker;
    use serde_json::{Value, json};
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    use super::*;
    use shared::data::{SecretContent, SecretFile};
    use shared::konst::LAB_FILE_NAME;

    use crate::services::lab_ssh_key::create_lab_ssh_key;
    use crate::services::node_secrets::write_node_secrets;

    /// A libvirt URI whose socket does not exist, so connecting always fails.
    const UNREACHABLE_QEMU_URI: &str = "qemu+unix:///system?socket=/nonexistent/sherpa-test.sock";

    const LAB_ID: &str = "fecb1a12";

    #[test]
    fn containers_are_the_labs_only_by_whole_name() {
        let names = |name: &str| vec![format!("/{name}")];
        assert_eq!(
            lab_container_name(&names("r1-fecb1a12"), LAB_ID),
            Some("r1-fecb1a12")
        );
        assert_eq!(
            lab_container_name(&names("sherpa-router-fecb1a12"), LAB_ID),
            Some("sherpa-router-fecb1a12")
        );
        // Another lab's node that is named after this lab's ID.
        assert_eq!(
            lab_container_name(&names("fecb1a12-0a1b2c3d"), LAB_ID),
            None
        );
        assert_eq!(lab_container_name(&names("r1-fecb1a12x"), LAB_ID), None);
    }

    #[test]
    fn only_the_labs_own_host_interfaces_are_deleted() {
        for name in [
            "tpa3-fecb1a12",
            "tpb3-fecb1a12",
            "bri1-fecb1a12",
            "brs2-fecb1a12",
            "cd1i2-fecb1a12",
        ] {
            assert!(is_lab_interface(name, LAB_ID), "{name}");
        }
        for name in [
            // Another lab's, though they contain this lab's ID.
            "tpa3-0a1b2c3d",
            "tpafecb1a12-x",
            // This lab's, but libvirt or Docker remove them.
            "brm-fecb1a12",
            "vnet3",
        ] {
            assert!(!is_lab_interface(name, LAB_ID), "{name}");
        }
    }
    #[test]
    fn lab_secrets_are_removed_with_the_lab() {
        let root = tempdir().unwrap();
        let file = SecretFile {
            name: "password".to_string(),
            mode: 444,
            content: SecretContent::from_bytes(b"pw"),
        };
        write_node_secrets(root.path(), "test1234", "db01", &[file]).unwrap();
        let mut errors = Vec::new();
        destroy_lab_secrets("test1234", root.path(), &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!root.path().join("test1234").exists());
    }

    struct DockerMock {
        docker: Docker,
        requests: Arc<Mutex<Vec<String>>>,
        task: JoinHandle<()>,
    }

    impl DockerMock {
        async fn new(containers: Value, gateway_failure: bool, volume_failure: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let captured = Arc::clone(&requests);
            let router = Router::new().fallback(move |method: Method, uri: Uri| {
                let captured = Arc::clone(&captured);
                let containers = containers.clone();
                async move {
                    captured.lock().unwrap().push(format!("{method} {uri}"));
                    let path = uri.path();
                    let (status, body) = if path.ends_with("/containers/json") {
                        (StatusCode::OK, containers)
                    } else if method == Method::GET
                        && path.ends_with("/containers/sherpa-tailnet-test1234/json")
                        && gateway_failure
                    {
                        (
                            StatusCode::OK,
                            json!({"Id":"gateway-id","Config":{"Labels":{
                                "sh.erpa.tailscale.lab":"test1234",
                                "sh.erpa.role":"tailnet-gateway"
                            }}}),
                        )
                    } else if method == Method::GET
                        && path.ends_with("/volumes/sherpa-tailnet-state-test1234")
                        && volume_failure
                    {
                        (
                            StatusCode::OK,
                            json!({"Name":"sherpa-tailnet-state-test1234",
                                "Driver":"local","Mountpoint":"/mock/volume",
                                "Options":{},"Scope":"local","Labels":{
                                "sh.erpa.tailscale.lab":"test1234",
                                "sh.erpa.role":"tailnet-gateway"
                            }}),
                        )
                    } else if method == Method::DELETE
                        && ((path.ends_with("/containers/gateway-id") && gateway_failure)
                            || (path.ends_with("/volumes/sherpa-tailnet-state-test1234")
                                && volume_failure))
                    {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            json!({"message":"cleanup failed"}),
                        )
                    } else if method == Method::DELETE || path.ends_with("/kill") {
                        (StatusCode::NO_CONTENT, Value::Null)
                    } else {
                        (StatusCode::NOT_FOUND, json!({"message":"missing"}))
                    };
                    (status, Json(body))
                }
            });
            let task = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let docker = Docker::connect_with_http(
                &format!("http://{address}"),
                5,
                bollard::API_DEFAULT_VERSION,
            )
            .unwrap();
            Self {
                docker,
                requests,
                task,
            }
        }
    }

    impl Drop for DockerMock {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    #[tokio::test]
    async fn cleanup_removes_ordinary_nodes_with_tailnet_names() {
        let mock = DockerMock::new(
            json!([
                {"Id":"node-id","Names":["/sherpa-tailnet-app-test1234"],"Labels":{}},
                {"Id":"exact-name-node","Names":["/sherpa-tailnet-test1234"],"Labels":{}}
            ]),
            false,
            false,
        )
        .await;
        let mut summary = DestroySummary::default();
        let mut errors = Vec::new();
        destroy_containers(
            "test1234",
            &mock.docker,
            &TailscaleGatewaySettings::default(),
            &mut summary,
            &mut errors,
        )
        .await;
        assert_eq!(
            summary.containers_destroyed,
            vec!["sherpa-tailnet-app-test1234", "sherpa-tailnet-test1234"]
        );
        assert!(errors.is_empty());
        let requests = mock.requests.lock().unwrap();
        let node_removal = requests
            .iter()
            .position(|request| {
                request.starts_with("DELETE ")
                    && request.contains("/containers/sherpa-tailnet-test1234?")
            })
            .unwrap();
        let gateway_inspection = requests
            .iter()
            .position(|request| request.contains("/containers/sherpa-tailnet-test1234/json"))
            .unwrap();
        assert!(node_removal < gateway_inspection);
    }

    #[tokio::test]
    async fn generic_cleanup_preserves_gateways_identified_by_labels() {
        let mock = DockerMock::new(
            json!([{"Id":"foreign-id","Names":["/renamed-test1234-gateway"],"Labels":{
                "sh.erpa.tailscale.lab":"another",
                "sh.erpa.role":"tailnet-gateway"
            }}]),
            false,
            false,
        )
        .await;
        let mut summary = DestroySummary::default();
        let mut errors = Vec::new();
        destroy_containers(
            "test1234",
            &mock.docker,
            &TailscaleGatewaySettings::default(),
            &mut summary,
            &mut errors,
        )
        .await;
        assert_eq!(summary.containers_destroyed, [] as [std::string::String; 0]);
        assert!(errors.is_empty());
        assert!(
            mock.requests
                .lock()
                .unwrap()
                .iter()
                .all(|request| !request.starts_with("DELETE ") && !request.contains("/kill"))
        );
    }

    async fn assert_gateway_cleanup_can_be_retried(gateway_failure: bool, volume_failure: bool) {
        let root = tempdir().unwrap();
        let lab_dir = root.path().join("test1234");
        fs::create_dir(&lab_dir).unwrap();
        let lab_file = lab_dir.join(LAB_FILE_NAME);
        let manifest_file = lab_dir.join("manifest.toml");
        fs::write(&lab_file, "saved lab info").unwrap();
        fs::write(&manifest_file, "name='test'\nnodes=[]").unwrap();
        let database_deleted = AtomicBool::new(false);
        let mock = DockerMock::new(json!([]), gateway_failure, volume_failure).await;
        let mut summary = DestroySummary::default();
        let mut errors = Vec::new();
        destroy_containers(
            "test1234",
            &mock.docker,
            &TailscaleGatewaySettings::default(),
            &mut summary,
            &mut errors,
        )
        .await;
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].resource_type, "tailscale");
        let failed_resource = if gateway_failure {
            "/containers/gateway-id"
        } else {
            "/volumes/sherpa-tailnet-state-test1234"
        };
        assert!(
            mock.requests
                .lock()
                .unwrap()
                .iter()
                .any(|request| request.starts_with("DELETE ") && request.contains(failed_resource))
        );
        cleanup_lab_metadata(
            "test1234",
            lab_dir.to_str().unwrap(),
            async {
                database_deleted.store(true, Ordering::SeqCst);
                Ok(())
            },
            &mut summary,
            &mut errors,
            None,
        )
        .await;
        assert!(!database_deleted.load(Ordering::SeqCst));
        assert!(!summary.database_records_deleted);
        assert!(!summary.lab_directory_deleted);
        assert_eq!(fs::read_to_string(&lab_file).unwrap(), "saved lab info");
        assert_eq!(
            fs::read_to_string(&manifest_file).unwrap(),
            "name='test'\nnodes=[]"
        );

        let retry = DockerMock::new(json!([]), false, false).await;
        let mut summary = DestroySummary::default();
        let mut errors = Vec::new();
        destroy_containers(
            "test1234",
            &retry.docker,
            &TailscaleGatewaySettings::default(),
            &mut summary,
            &mut errors,
        )
        .await;
        cleanup_lab_metadata(
            "test1234",
            lab_dir.to_str().unwrap(),
            async {
                database_deleted.store(true, Ordering::SeqCst);
                Ok(())
            },
            &mut summary,
            &mut errors,
            None,
        )
        .await;
        assert!(database_deleted.load(Ordering::SeqCst));
        assert!(summary.database_records_deleted);
        assert!(summary.lab_directory_deleted);
        assert!(!lab_dir.exists());
        assert!(errors.is_empty());
    }

    #[tokio::test]
    async fn failed_gateway_removal_retains_metadata_until_retry() {
        assert_gateway_cleanup_can_be_retried(true, false).await;
    }

    #[tokio::test]
    async fn failed_identity_volume_removal_retains_metadata_until_retry() {
        assert_gateway_cleanup_can_be_retried(false, true).await;
    }
    #[tokio::test]
    async fn any_resource_cleanup_failure_retains_metadata_until_retry() {
        for resource_type in [
            "vm",
            "disk",
            "container",
            "docker_network",
            "libvirt_network",
            "interface",
            "secrets",
        ] {
            let root = tempdir().unwrap();
            let lab_dir = root.path().join("lab");
            fs::create_dir(&lab_dir).unwrap();
            fs::write(lab_dir.join(LAB_FILE_NAME), "saved metadata").unwrap();
            let database_deleted = AtomicBool::new(false);
            let mut summary = DestroySummary::default();
            let mut errors = vec![DestroyError::new(
                resource_type,
                "resource",
                "cleanup failed",
            )];
            cleanup_lab_metadata(
                "lab",
                lab_dir.to_str().unwrap(),
                async {
                    database_deleted.store(true, Ordering::SeqCst);
                    Ok(())
                },
                &mut summary,
                &mut errors,
                None,
            )
            .await;
            assert!(!database_deleted.load(Ordering::SeqCst));
            assert!(lab_dir.join(LAB_FILE_NAME).exists());
            errors.clear();
            cleanup_lab_metadata(
                "lab",
                lab_dir.to_str().unwrap(),
                async {
                    database_deleted.store(true, Ordering::SeqCst);
                    Ok(())
                },
                &mut summary,
                &mut errors,
                None,
            )
            .await;
            assert!(database_deleted.load(Ordering::SeqCst));
            assert!(!lab_dir.exists());
        }
    }

    #[tokio::test]
    async fn libvirt_connection_failure_is_recorded_and_retains_metadata() {
        let qemu = libvirt::Qemu {
            uri: UNREACHABLE_QEMU_URI.to_string(),
        };
        let mut summary = DestroySummary::default();
        let mut errors = Vec::new();

        destroy_vms_and_disks("lab", &qemu, &mut summary, &mut errors);
        destroy_libvirt_networks("lab", &qemu, &mut summary, &mut errors);

        let resource_types: Vec<&str> = errors
            .iter()
            .map(|error| error.resource_type.as_str())
            .collect();
        assert_eq!(resource_types, vec!["vm", "libvirt_network"]);
        assert!(
            errors
                .iter()
                .all(|error| error.error_message.contains("Failed to connect to libvirt")),
            "{errors:?}"
        );

        // VMs and disks may be left, so the lab is kept for a destroy retry.
        let root = tempdir().unwrap();
        let lab_dir = root.path().join("lab");
        fs::create_dir(&lab_dir).unwrap();
        let database_deleted = AtomicBool::new(false);
        cleanup_lab_metadata(
            "lab",
            lab_dir.to_str().unwrap(),
            async {
                database_deleted.store(true, Ordering::SeqCst);
                Ok(())
            },
            &mut summary,
            &mut errors,
            None,
        )
        .await;
        assert!(!database_deleted.load(Ordering::SeqCst));
        assert!(lab_dir.exists());
    }

    #[tokio::test]
    async fn the_labs_ssh_key_is_deleted_with_the_lab_folder() {
        let root = tempdir().unwrap();
        let lab_dir = root.path().join("test1234");
        fs::create_dir(&lab_dir).unwrap();
        let key = create_lab_ssh_key(lab_dir.to_str().unwrap()).unwrap();
        let mut summary = DestroySummary::default();
        let mut errors = Vec::new();

        cleanup_lab_metadata(
            "test1234",
            lab_dir.to_str().unwrap(),
            async { Ok(()) },
            &mut summary,
            &mut errors,
            None,
        )
        .await;

        assert!(errors.is_empty(), "{errors:?}");
        assert!(summary.lab_directory_deleted);
        assert!(!Path::new(&key.private).exists());
        assert!(!Path::new(&key.public).exists());
    }

    #[tokio::test]
    async fn the_lab_folder_goes_before_the_lab_record() {
        // M7.1: the record used to go first, and a folder that then could not be removed was
        // left with no record, which neither destroy nor clean could reach.
        let root = tempdir().unwrap();
        let lab_dir = root.path().join("lab");
        fs::create_dir(&lab_dir).unwrap();
        fs::write(lab_dir.join(LAB_FILE_NAME), "saved lab info").unwrap();
        let folder_gone_first = AtomicBool::new(false);
        let mut summary = DestroySummary::default();
        let mut errors = Vec::new();

        cleanup_lab_metadata(
            "lab",
            lab_dir.to_str().unwrap(),
            async {
                folder_gone_first.store(!lab_dir.exists(), Ordering::SeqCst);
                Ok(())
            },
            &mut summary,
            &mut errors,
            None,
        )
        .await;

        assert!(errors.is_empty(), "{errors:?}");
        assert!(folder_gone_first.load(Ordering::SeqCst));
        assert!(summary.lab_directory_deleted && summary.database_records_deleted);
    }

    #[tokio::test]
    async fn a_lab_folder_that_cannot_be_removed_keeps_the_lab_record() {
        // M7.1: the owner can destroy the lab again only while its record exists.
        let root = tempdir().unwrap();
        let lab_dir = root.path().join("lab");
        fs::create_dir(&lab_dir).unwrap();
        fs::write(lab_dir.join(LAB_FILE_NAME), "saved lab info").unwrap();
        fs::set_permissions(&lab_dir, fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(lab_dir.join("probe"), "").is_ok() {
            // Running as root: permissions do not stop the removal, so there is nothing to test.
            return;
        }
        let database_deleted = AtomicBool::new(false);
        let mut summary = DestroySummary::default();
        let mut errors = Vec::new();

        cleanup_lab_metadata(
            "lab",
            lab_dir.to_str().unwrap(),
            async {
                database_deleted.store(true, Ordering::SeqCst);
                Ok(())
            },
            &mut summary,
            &mut errors,
            None,
        )
        .await;

        fs::set_permissions(&lab_dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!database_deleted.load(Ordering::SeqCst));
        assert!(!summary.database_records_deleted);
        assert_eq!(
            errors
                .iter()
                .map(|e| e.resource_type.as_str())
                .collect::<Vec<_>>(),
            ["filesystem"]
        );
    }
}
