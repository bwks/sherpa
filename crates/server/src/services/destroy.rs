use std::fs;

use anyhow::{Context, Result, anyhow};
use virt::storage_pool::StoragePool;
use virt::sys::VIR_DOMAIN_UNDEFINE_NVRAM;

use container::{delete_network, kill_container, list_containers, list_networks};
use libvirt::delete_disk;
use network::{delete_interface, find_interfaces_fuzzy};
use shared::data::{
    DestroyError, DestroyRequest, DestroyResponse, DestroySummary, LabInfo, StatusKind,
};
use shared::konst::{
    BRIDGE_PREFIX, LAB_FILE_NAME, SHERPA_BASE_DIR, SHERPA_LABS_DIR, SHERPA_STORAGE_POOL,
    SHERPA_STORAGE_POOL_PATH, VETH_PREFIX,
};
use shared::util::{dir_exists, file_exists, load_file};
use std::str::FromStr;

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
/// tracking successes and failures separately.
///
/// TODO: Currently accepts username without authentication. This assumes a trusted
/// environment where the client can be trusted to send correct username. In production,
/// this should be replaced with proper authentication (JWT, session, etc.) where the
/// username is extracted from a verified token rather than client-provided param.
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
    let lab_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_LABS_DIR}/{lab_id}");
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

    // 6. Clean up database
    tracing::info!(lab_id = %lab_id, "Cleaning up database records");
    let _ = progress.send_status(
        "Cleaning up database records...".to_string(),
        StatusKind::Progress,
    );
    match cleanup_database(lab_id, &state.db).await {
        Ok(_) => {
            summary.database_records_deleted = true;
            let _ = progress.send_status("Database records cleaned".to_string(), StatusKind::Done);
            tracing::info!(lab_id = %lab_id, "Database cleanup successful");
        }
        Err(e) => {
            summary.database_records_deleted = false;
            errors.push(DestroyError::new("database", lab_id, format!("{:?}", e)));
            tracing::error!(lab_id = %lab_id, error = ?e, "Database cleanup failed");
        }
    }

    // 7. Delete lab directory
    tracing::info!(lab_id = %lab_id, lab_dir = %lab_dir, "Deleting lab directory");
    let _ = progress.send_status(
        "Deleting lab directory...".to_string(),
        StatusKind::Progress,
    );
    if dir_exists(&lab_dir) {
        match fs::remove_dir_all(&lab_dir) {
            Ok(_) => {
                summary.lab_directory_deleted = true;
                let _ = progress.send_status("Lab directory deleted".to_string(), StatusKind::Done);
                tracing::info!(lab_id = %lab_id, lab_dir = %lab_dir, "Lab directory deleted");
            }
            Err(e) => {
                summary.lab_directory_deleted = false;
                errors.push(DestroyError::new(
                    "filesystem",
                    &lab_dir,
                    format!("{:?}", e),
                ));
                tracing::error!(lab_id = %lab_id, lab_dir = %lab_dir, error = ?e, "Failed to delete lab directory");
            }
        }
    } else {
        // Directory doesn't exist - consider it success (idempotent)
        summary.lab_directory_deleted = true;
        let _ = progress.send_status("Lab directory deleted".to_string(), StatusKind::Done);
        tracing::debug!(lab_id = %lab_id, lab_dir = %lab_dir, "Lab directory already removed");
    }

    // Determine overall success
    let success = errors.is_empty();
    let total_duration = start_time.elapsed().as_secs();

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

/// Destroy all containers for a lab
pub(crate) async fn destroy_containers(
    lab_id: &str,
    docker: &bollard::Docker,
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
                            tracing::debug!(
                                lab_id = %lab_id,
                                container_name = %name,
                                "Destroying container"
                            );
                            match kill_container(docker, name).await {
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
                                        "Failed to destroy container"
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
}

/// Destroy all VMs and their disks for a lab
pub(crate) fn destroy_vms_and_disks(
    lab_id: &str,
    qemu: &libvirt::Qemu,
    summary: &mut DestroySummary,
    errors: &mut Vec<DestroyError>,
) -> Result<()> {
    let qemu_conn = qemu.connect().context("Failed to connect to libvirt")?;
    let domains = qemu_conn
        .list_all_domains(0)
        .context("Failed to list domains")?;
    let storage_pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL).context(
        format!("Failed to find storage pool '{}'", SHERPA_STORAGE_POOL),
    )?;
    let pool_disks = storage_pool
        .list_volumes()
        .context("Failed to list storage volumes")?;

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

        if vm_name.contains(lab_id) {
            let is_active = domain.is_active().unwrap_or(false);

            // Destroy the VM
            match (|| -> Result<()> {
                // UEFI domains will have an NVRAM file that must be deleted.
                let nvram_flag = VIR_DOMAIN_UNDEFINE_NVRAM;
                domain
                    .undefine_flags(nvram_flag)
                    .context("Failed to undefine domain")?;
                if is_active {
                    domain.destroy().context("Failed to destroy domain")?;
                }
                Ok(())
            })() {
                Ok(_) => {
                    summary.vms_destroyed.push(vm_name.clone());
                    tracing::info!("Destroyed VM: {}", vm_name);

                    // Destroy associated disks
                    let domain_disks: Vec<&String> = pool_disks
                        .iter()
                        .filter(|d| d.starts_with(&vm_name))
                        .collect();

                    for disk in domain_disks {
                        if file_exists(&format!("{SHERPA_STORAGE_POOL_PATH}/{disk}")) {
                            match delete_disk(&qemu_conn, disk) {
                                Ok(_) => {
                                    summary.disks_deleted.push(disk.to_string());
                                    tracing::info!("Deleted disk: {}", disk);
                                }
                                Err(e) => {
                                    summary.disks_failed.push(disk.to_string());
                                    errors.push(DestroyError::new(
                                        "disk",
                                        disk,
                                        format!("{:?}", e),
                                    ));
                                    tracing::error!("Failed to delete disk {}: {:?}", disk, e);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    summary.vms_failed.push(vm_name.clone());
                    errors.push(DestroyError::new("vm", &vm_name, format!("{:?}", e)));
                    tracing::error!("Failed to destroy VM {}: {:?}", vm_name, e);
                }
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
                network.destroy().context("Failed to destroy network")?;
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
pub(crate) async fn cleanup_database(
    lab_id: &str,
    db: &std::sync::Arc<surrealdb::Surreal<surrealdb::engine::remote::ws::Client>>,
) -> Result<()> {
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
