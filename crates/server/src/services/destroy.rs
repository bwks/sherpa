use std::fs;
use std::future::Future;

use anyhow::{Context, Result, anyhow};
use opentelemetry::KeyValue;
use virt::storage_pool::StoragePool;
use virt::sys::VIR_DOMAIN_UNDEFINE_NVRAM;

use container::tailscale::{gateway_name, is_gateway, remove_gateway};
use container::{
    Docker, delete_network, kill_container, list_containers, list_networks, remove_container,
};
use libvirt::delete_disk;
use network::{delete_interface, find_interfaces_fuzzy};
use shared::data::{
    DestroyError, DestroyRequest, DestroyResponse, DestroySummary, LabInfo, StatusKind,
};
use shared::konst::{
    BRIDGE_PREFIX, CONTAINER_VETH_PREFIX, LAB_FILE_NAME, SHERPA_LABS_PATH, SHERPA_STORAGE_POOL,
    TAP_PREFIX, VETH_PREFIX,
};
use shared::util::{dir_exists, load_file};
use std::str::FromStr;

use tracing::instrument;

use crate::daemon::state::AppState;
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
/// TODO: Currently accepts username without authentication. This assumes a trusted
/// environment where the client can be trusted to send correct username. In production,
/// this should be replaced with proper authentication (JWT, session, etc.) where the
/// username is extracted from a verified token rather than client-provided param.
#[instrument(skip(state, progress), fields(lab_id = %request.lab_id))]
pub async fn destroy_lab(
    request: DestroyRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<DestroyResponse> {
    let lab_id = &request.lab_id;
    let username = &request.username;

    let start_time = std::time::Instant::now();

    tracing::info!(
        lab_id = %lab_id,
        username = %username,
        "Starting lab destruction"
    );

    let mut summary = DestroySummary::default();
    let mut errors = Vec::new();

    // Get user from database to validate existence and get RecordId
    let db_user = db::get_user(&state.db, username)
        .await
        .context(format!("User '{}' not found in database", username))?;

    let user_id = db_user
        .id
        .ok_or_else(|| anyhow!("User '{}' missing record ID", username))?;

    // Get lab from database
    let db_lab = db::get_lab(&state.db, lab_id)
        .await
        .context(format!("Lab '{}' not found in database", lab_id))?;

    // Validate ownership
    if db_lab.user != user_id {
        tracing::warn!(
            lab_id = %lab_id,
            username = %username,
            "Permission denied - lab owned by different user"
        );
        return Err(anyhow!(
            "Permission denied: Lab '{}' is owned by another user",
            lab_id
        ));
    }

    // Load lab info from filesystem
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let lab_file = load_file(&format!("{lab_dir}/{LAB_FILE_NAME}"))
        .context("Unable to load lab file. Is the lab running?")?;
    let lab_info = LabInfo::from_str(&lab_file).context("Failed to parse lab info file")?;

    let lab_name = lab_info.name.clone();

    tracing::debug!(
        lab_id = %lab_id,
        lab_name = %lab_name,
        lab_dir = %lab_dir,
        "Loaded lab information"
    );

    // 1. Destroy containers
    let containers_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Destroying containers");
    let _ = progress.send_status("Destroying containers...".to_string(), StatusKind::Progress);
    destroy_containers(lab_id, &state.docker, &mut summary, &mut errors).await;
    let containers_duration = containers_timer.elapsed().as_secs();
    if summary.containers_destroyed.is_empty() && summary.containers_failed.is_empty() {
        let _ = progress.send_status("No containers to destroy".to_string(), StatusKind::Info);
    } else {
        for name in &summary.containers_destroyed {
            let _ =
                progress.send_status(format!("Destroyed container: {}", name), StatusKind::Done);
        }
    }
    tracing::info!(
        lab_id = %lab_id,
        destroyed = summary.containers_destroyed.len(),
        failed = summary.containers_failed.len(),
        duration_secs = containers_duration,
        "Container destruction completed"
    );

    // 2. Destroy VMs and disks
    let vms_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Destroying VMs and disks");
    let _ = progress.send_status(
        "Destroying VMs and disks...".to_string(),
        StatusKind::Progress,
    );
    destroy_vms_and_disks(lab_id, &state.qemu, &mut summary, &mut errors)?;
    if summary.vms_destroyed.is_empty() && summary.vms_failed.is_empty() {
        let _ = progress.send_status("No VMs to destroy".to_string(), StatusKind::Info);
    } else {
        for name in &summary.vms_destroyed {
            let _ = progress.send_status(format!("Destroyed VM: {}", name), StatusKind::Done);
        }
        for name in &summary.disks_deleted {
            let _ = progress.send_status(format!("Deleted disk: {}", name), StatusKind::Done);
        }
    }
    let vms_duration = vms_timer.elapsed().as_secs();
    tracing::info!(
        lab_id = %lab_id,
        vms_destroyed = summary.vms_destroyed.len(),
        vms_failed = summary.vms_failed.len(),
        disks_deleted = summary.disks_deleted.len(),
        disks_failed = summary.disks_failed.len(),
        duration_secs = vms_duration,
        "VM and disk destruction completed"
    );

    // 3. Destroy Docker networks
    let docker_net_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Destroying Docker networks");
    let _ = progress.send_status(
        "Destroying Docker networks...".to_string(),
        StatusKind::Progress,
    );
    destroy_docker_networks(lab_id, &state.docker, &mut summary, &mut errors).await;
    if summary.docker_networks_destroyed.is_empty() && summary.docker_networks_failed.is_empty() {
        let _ = progress.send_status(
            "No Docker networks to destroy".to_string(),
            StatusKind::Info,
        );
    } else {
        for name in &summary.docker_networks_destroyed {
            let _ = progress.send_status(
                format!("Destroyed Docker network: {}", name),
                StatusKind::Done,
            );
        }
    }
    let docker_net_duration = docker_net_timer.elapsed().as_secs();
    tracing::info!(
        lab_id = %lab_id,
        destroyed = summary.docker_networks_destroyed.len(),
        failed = summary.docker_networks_failed.len(),
        duration_secs = docker_net_duration,
        "Docker network destruction completed"
    );

    // 4. Destroy libvirt networks
    let libvirt_net_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Destroying libvirt networks");
    let _ = progress.send_status(
        "Destroying libvirt networks...".to_string(),
        StatusKind::Progress,
    );
    destroy_libvirt_networks(lab_id, &state.qemu, &mut summary, &mut errors)?;
    if summary.libvirt_networks_destroyed.is_empty() && summary.libvirt_networks_failed.is_empty() {
        let _ = progress.send_status(
            "No libvirt networks to destroy".to_string(),
            StatusKind::Info,
        );
    } else {
        for name in &summary.libvirt_networks_destroyed {
            let _ = progress.send_status(
                format!("Destroyed libvirt network: {}", name),
                StatusKind::Done,
            );
        }
    }
    let libvirt_net_duration = libvirt_net_timer.elapsed().as_secs();
    tracing::info!(
        lab_id = %lab_id,
        destroyed = summary.libvirt_networks_destroyed.len(),
        failed = summary.libvirt_networks_failed.len(),
        duration_secs = libvirt_net_duration,
        "Libvirt network destruction completed"
    );

    // 5. Delete network interfaces
    let interfaces_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Deleting network interfaces");
    let _ = progress.send_status(
        "Deleting network interfaces...".to_string(),
        StatusKind::Progress,
    );
    destroy_interfaces(lab_id, &mut summary, &mut errors).await;
    if summary.interfaces_deleted.is_empty() && summary.interfaces_failed.is_empty() {
        let _ = progress.send_status(
            "No network interfaces to delete".to_string(),
            StatusKind::Info,
        );
    } else {
        for name in &summary.interfaces_deleted {
            let _ = progress.send_status(format!("Deleted interface: {}", name), StatusKind::Done);
        }
    }
    let interfaces_duration = interfaces_timer.elapsed().as_secs();
    tracing::info!(
        lab_id = %lab_id,
        deleted = summary.interfaces_deleted.len(),
        failed = summary.interfaces_failed.len(),
        duration_secs = interfaces_duration,
        "Network interface deletion completed"
    );

    cleanup_lab_metadata(
        lab_id,
        &lab_dir,
        cleanup_database(lab_id, &state.db),
        &mut summary,
        &mut errors,
        Some(&progress),
    )
    .await;

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

/// Finish database and filesystem cleanup after infrastructure teardown.
/// Keep metadata when Tailscale resources remain; the database future is only
/// polled once their cleanup succeeds.
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
    if errors
        .iter()
        .any(|error| error.resource_type == "tailscale")
    {
        tracing::warn!(lab_id = %lab_id, "Retaining lab ownership and saved configuration after Tailscale cleanup failure");
        send_status(
            "Tailscale cleanup failed; lab ownership and saved configuration retained for a cleanup retry."
                .to_string(),
            StatusKind::Info,
        );
        return;
    }

    tracing::info!(lab_id = %lab_id, "Cleaning up database records");
    send_status(
        "Cleaning up database records...".to_string(),
        StatusKind::Progress,
    );
    match database_cleanup.await {
        Ok(_) => {
            summary.database_records_deleted = true;
            send_status("Database records cleaned".to_string(), StatusKind::Done);
            tracing::info!(lab_id = %lab_id, "Database cleanup successful");
        }
        Err(e) => {
            summary.database_records_deleted = false;
            errors.push(DestroyError::new("database", lab_id, format!("{:?}", e)));
            tracing::error!(lab_id = %lab_id, error = ?e, "Database cleanup failed");
        }
    }

    tracing::info!(lab_id = %lab_id, lab_dir = %lab_dir, "Deleting lab directory");
    send_status(
        "Deleting lab directory...".to_string(),
        StatusKind::Progress,
    );
    if dir_exists(lab_dir) {
        match fs::remove_dir_all(lab_dir) {
            Ok(_) => {
                summary.lab_directory_deleted = true;
                send_status("Lab directory deleted".to_string(), StatusKind::Done);
                tracing::info!(lab_id = %lab_id, lab_dir = %lab_dir, "Lab directory deleted");
            }
            Err(e) => {
                summary.lab_directory_deleted = false;
                errors.push(DestroyError::new("filesystem", lab_dir, format!("{:?}", e)));
                tracing::error!(lab_id = %lab_id, lab_dir = %lab_dir, error = ?e, "Failed to delete lab directory");
            }
        }
    } else {
        // Directory doesn't exist - consider it success (idempotent)
        summary.lab_directory_deleted = true;
        send_status("Lab directory deleted".to_string(), StatusKind::Done);
        tracing::debug!(lab_id = %lab_id, lab_dir = %lab_dir, "Lab directory already removed");
    }
}

/// Destroy all containers for a lab
#[instrument(skip(docker, summary, errors), fields(%lab_id), level = "debug")]
pub(crate) async fn destroy_containers(
    lab_id: &str,
    docker: &Docker,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
) {
    match list_containers(docker).await {
        Ok(containers) => {
            let lab_containers: Vec<_> = containers
                .iter()
                .filter(|c| {
                    c.names
                        .as_ref()
                        .is_some_and(|names| names.iter().any(|name| name.contains(lab_id)))
                })
                .collect();

            tracing::debug!(
                lab_id = %lab_id,
                container_count = lab_containers.len(),
                "Found containers to destroy"
            );

            for container in containers {
                if let Some(names) = &container.names {
                    // Check if any container name contains the lab_id
                    if names.iter().any(|name| name.contains(lab_id)) {
                        // From docs: for historical reasons, container names start with a '/'
                        // Extract the actual container name (remove leading /)
                        if let Some(container_name) = names.first() {
                            let name = container_name.trim_start_matches('/');
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
                                Ok(_) => {
                                    summary.containers_destroyed.push(name.to_string());
                                    tracing::info!(
                                        lab_id = %lab_id,
                                        container_name = %name,
                                        "Container destroyed"
                                    );
                                }
                                Err(e) => {
                                    summary.containers_failed.push(name.to_string());
                                    errors.push(DestroyError::new(
                                        "container",
                                        name,
                                        format!("{:?}", e),
                                    ));
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
                }
            }
        }
        Err(e) => {
            errors.push(DestroyError::new(
                "container",
                "list_containers",
                format!("Failed to list containers: {:?}", e),
            ));
            tracing::error!(lab_id = %lab_id, error = ?e, "Failed to list containers");
        }
    }

    // Remove ordinary nodes first, including nodes named like the gateway.
    // Gateway cleanup still runs if listing or removing ordinary nodes fails.
    if let Err(error) = remove_gateway(docker, lab_id).await {
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

/// Destroy all VMs and their disks for a lab
pub(crate) fn destroy_vms_and_disks(
    lab_id: &str,
    qemu: &libvirt::Qemu,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
) -> Result<()> {
    let qemu_conn = qemu.connect().context("Failed to connect to libvirt")?;

    // Destroy VMs
    let domains = qemu_conn
        .list_all_domains(0)
        .context("Failed to list domains")?;

    for domain in domains {
        let vm_name = match domain.get_name() {
            Ok(name) => name,
            Err(e) => {
                errors.push(DestroyError::new(
                    "vm",
                    "unknown",
                    format!("Failed to get domain name: {:?}", e),
                ));
                continue;
            }
        };

        if !vm_name.contains(lab_id) {
            continue;
        }

        let is_active = domain.is_active().unwrap_or(false);

        // UEFI domains will have an NVRAM file that must be deleted.
        let result = domain
            .undefine_flags(VIR_DOMAIN_UNDEFINE_NVRAM)
            .context("Failed to undefine domain")
            .and_then(|_| {
                if is_active {
                    domain.destroy().context("Failed to destroy domain")?;
                }
                Ok(())
            });

        match result {
            Ok(_) => {
                summary.vms_destroyed.push(vm_name.clone());
                tracing::info!("Destroyed VM: {}", vm_name);
            }
            Err(e) => {
                summary.vms_failed.push(vm_name.clone());
                errors.push(DestroyError::new("vm", &vm_name, format!("{:?}", e)));
                tracing::error!("Failed to destroy VM {}: {:?}", vm_name, e);
            }
        }
    }

    // Delete all disks belonging to this lab

    let storage_pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL).context(
        format!("Failed to find storage pool '{}'", SHERPA_STORAGE_POOL),
    )?;
    let pool_disks = storage_pool
        .list_volumes()
        .context("Failed to list storage volumes")?;

    let lab_disks: Vec<&String> = pool_disks.iter().filter(|d| d.contains(lab_id)).collect();

    for disk in lab_disks {
        match delete_disk(&qemu_conn, disk) {
            Ok(_) => {
                summary.disks_deleted.push(disk.to_string());
                tracing::info!("Deleted disk: {}", disk);
            }
            Err(e) => {
                summary.disks_failed.push(disk.to_string());
                errors.push(DestroyError::new("disk", disk, format!("{:?}", e)));
                tracing::error!("Failed to delete disk {}: {:?}", disk, e);
            }
        }
    }

    Ok(())
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
                    && network_name.contains(lab_id)
                {
                    match delete_network(docker, &network_name).await {
                        Ok(_) => {
                            summary.docker_networks_destroyed.push(network_name.clone());
                            tracing::info!("Destroyed Docker network: {}", network_name);
                        }
                        Err(e) => {
                            summary.docker_networks_failed.push(network_name.clone());
                            errors.push(DestroyError::new(
                                "docker_network",
                                &network_name,
                                format!("{:?}", e),
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
                format!("Failed to list Docker networks: {:?}", e),
            ));
            tracing::error!("Failed to list Docker networks: {:?}", e);
        }
    }
}

/// Destroy all libvirt networks for a lab
pub(crate) fn destroy_libvirt_networks(
    lab_id: &str,
    qemu: &libvirt::Qemu,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
) -> Result<()> {
    let qemu_conn = qemu.connect().context("Failed to connect to libvirt")?;
    let networks = qemu_conn
        .list_all_networks(0)
        .context("Failed to list networks")?;

    for network in networks {
        let network_name = match network.get_name() {
            Ok(name) => name,
            Err(e) => {
                errors.push(DestroyError::new(
                    "libvirt_network",
                    "unknown",
                    format!("Failed to get network name: {:?}", e),
                ));
                continue;
            }
        };

        if network_name.contains(lab_id) {
            match (|| -> Result<()> {
                let is_active = network.is_active().unwrap_or(false);
                if is_active {
                    network.destroy().context("Failed to destroy network")?;
                }
                network.undefine().context("Failed to undefine network")?;
                Ok(())
            })() {
                Ok(_) => {
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
                        format!("{:?}", e),
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

    Ok(())
}

/// Delete network interfaces for a lab
pub(crate) async fn destroy_interfaces(
    lab_id: &str,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
) {
    match find_interfaces_fuzzy(lab_id).await {
        Ok(lab_interfaces) => {
            for interface in lab_interfaces {
                // Only delete interfaces created outside of Libvirt/Docker
                // Only 1 side of the veth interface needs to be deleted
                if interface.starts_with(&format!("{}a", BRIDGE_PREFIX))
                    || interface.starts_with(&format!("{}b", BRIDGE_PREFIX))
                    || interface.starts_with(&format!("{}i", BRIDGE_PREFIX))
                    || interface.starts_with(&format!("{}s", BRIDGE_PREFIX))
                    || interface.starts_with(&format!("{}a", VETH_PREFIX))
                    || interface.starts_with(&format!("{}a", TAP_PREFIX))
                    || interface.starts_with(&format!("{}b", TAP_PREFIX))
                    || interface.starts_with(CONTAINER_VETH_PREFIX)
                    || interface.starts_with("cd")
                    || interface.starts_with("ce")
                {
                    match delete_interface(&interface).await {
                        Ok(_) => {
                            summary.interfaces_deleted.push(interface.clone());
                            tracing::info!("Deleted interface: {}", interface);
                        }
                        Err(e) => {
                            summary.interfaces_failed.push(interface.clone());
                            errors.push(DestroyError::new(
                                "interface",
                                &interface,
                                format!("{:?}", e),
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
                format!("Failed to find interfaces: {:?}", e),
            ));
            tracing::error!("Failed to find interfaces: {:?}", e);
        }
    }
}

/// Clean up database records for a lab
pub(crate) async fn cleanup_database(lab_id: &str, db: &db::Database) -> Result<()> {
    db::delete_lab_links(db, lab_id)
        .await
        .context("Failed to delete lab links")?;
    db::delete_lab_nodes(db, lab_id)
        .await
        .context("Failed to delete lab nodes")?;
    db::delete_lab(db, lab_id)
        .await
        .context("Failed to delete lab")?;
    Ok(())
}

#[cfg(test)]
mod tests {
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
        destroy_containers("test1234", &mock.docker, &mut summary, &mut errors).await;
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
        destroy_containers("test1234", &mock.docker, &mut summary, &mut errors).await;
        assert!(summary.containers_destroyed.is_empty());
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
        destroy_containers("test1234", &mock.docker, &mut summary, &mut errors).await;
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
        destroy_containers("test1234", &retry.docker, &mut summary, &mut errors).await;
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
}
