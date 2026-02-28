use std::fs;

use anyhow::Result;

use shared::data::{DestroyError, DestroyResponse, DestroySummary};
use shared::konst::{LAB_FILE_NAME, SHERPA_BASE_DIR, SHERPA_LABS_DIR};
use shared::util::{dir_exists, load_file};

use crate::daemon::state::AppState;
use crate::services::destroy::{
    cleanup_database, destroy_containers, destroy_docker_networks, destroy_interfaces,
    destroy_libvirt_networks, destroy_vms_and_disks,
};

/// Clean all resources for a lab without ownership validation
///
/// Unlike `destroy_lab`, this function:
/// - Does not require the lab to exist in the database
/// - Does not validate user ownership (admin-only, verified at RPC layer)
/// - Tolerates missing lab info files
/// - Always attempts all resource types regardless of partial failures
pub async fn clean_lab(lab_id: &str, state: &AppState) -> Result<DestroyResponse> {
    let start_time = std::time::Instant::now();

    tracing::info!(lab_id = %lab_id, "Starting admin clean operation");

    let mut summary = DestroySummary::default();
    let mut errors = Vec::new();

    // Try to load lab name from filesystem, fall back to "unknown"
    let lab_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_LABS_DIR}/{lab_id}");
    let lab_name = load_file(&format!("{lab_dir}/{LAB_FILE_NAME}"))
        .ok()
        .and_then(|content| {
            content
                .parse::<shared::data::LabInfo>()
                .ok()
                .map(|info| info.name)
        })
        .unwrap_or_else(|| "unknown".to_string());

    tracing::debug!(
        lab_id = %lab_id,
        lab_name = %lab_name,
        lab_dir = %lab_dir,
        "Clean operation context"
    );

    // 1. Destroy containers
    let containers_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Cleaning containers");
    destroy_containers(lab_id, &state.docker, &mut summary, &mut errors).await;
    tracing::info!(
        lab_id = %lab_id,
        destroyed = summary.containers_destroyed.len(),
        failed = summary.containers_failed.len(),
        duration_secs = containers_timer.elapsed().as_secs(),
        "Container cleanup completed"
    );

    // 2. Destroy VMs and disks
    let vms_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Cleaning VMs and disks");
    if let Err(e) = destroy_vms_and_disks(lab_id, &state.qemu, &mut summary, &mut errors) {
        errors.push(DestroyError::new(
            "vm",
            lab_id,
            format!("VM/disk cleanup error: {:?}", e),
        ));
        tracing::error!(lab_id = %lab_id, error = ?e, "VM/disk cleanup encountered an error");
    }
    tracing::info!(
        lab_id = %lab_id,
        vms_destroyed = summary.vms_destroyed.len(),
        vms_failed = summary.vms_failed.len(),
        disks_deleted = summary.disks_deleted.len(),
        disks_failed = summary.disks_failed.len(),
        duration_secs = vms_timer.elapsed().as_secs(),
        "VM and disk cleanup completed"
    );

    // 3. Destroy Docker networks
    let docker_net_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Cleaning Docker networks");
    destroy_docker_networks(lab_id, &state.docker, &mut summary, &mut errors).await;
    tracing::info!(
        lab_id = %lab_id,
        destroyed = summary.docker_networks_destroyed.len(),
        failed = summary.docker_networks_failed.len(),
        duration_secs = docker_net_timer.elapsed().as_secs(),
        "Docker network cleanup completed"
    );

    // 4. Destroy libvirt networks
    let libvirt_net_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Cleaning libvirt networks");
    if let Err(e) = destroy_libvirt_networks(lab_id, &state.qemu, &mut summary, &mut errors) {
        errors.push(DestroyError::new(
            "libvirt_network",
            lab_id,
            format!("Libvirt network cleanup error: {:?}", e),
        ));
        tracing::error!(lab_id = %lab_id, error = ?e, "Libvirt network cleanup encountered an error");
    }
    tracing::info!(
        lab_id = %lab_id,
        destroyed = summary.libvirt_networks_destroyed.len(),
        failed = summary.libvirt_networks_failed.len(),
        duration_secs = libvirt_net_timer.elapsed().as_secs(),
        "Libvirt network cleanup completed"
    );

    // 5. Delete network interfaces
    let interfaces_timer = std::time::Instant::now();
    tracing::info!(lab_id = %lab_id, "Cleaning network interfaces");
    destroy_interfaces(lab_id, &mut summary, &mut errors).await;
    tracing::info!(
        lab_id = %lab_id,
        deleted = summary.interfaces_deleted.len(),
        failed = summary.interfaces_failed.len(),
        duration_secs = interfaces_timer.elapsed().as_secs(),
        "Network interface cleanup completed"
    );

    // 6. Clean up database (tolerate missing records)
    tracing::info!(lab_id = %lab_id, "Cleaning database records");
    match cleanup_database(lab_id, &state.db).await {
        Ok(_) => {
            summary.database_records_deleted = true;
            tracing::info!(lab_id = %lab_id, "Database cleanup successful");
        }
        Err(e) => {
            summary.database_records_deleted = false;
            errors.push(DestroyError::new("database", lab_id, format!("{:?}", e)));
            tracing::warn!(lab_id = %lab_id, error = ?e, "Database cleanup failed (may not exist)");
        }
    }

    // 7. Delete lab directory
    tracing::info!(lab_id = %lab_id, lab_dir = %lab_dir, "Deleting lab directory");
    if dir_exists(&lab_dir) {
        match fs::remove_dir_all(&lab_dir) {
            Ok(_) => {
                summary.lab_directory_deleted = true;
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
        summary.lab_directory_deleted = true;
        tracing::debug!(lab_id = %lab_id, lab_dir = %lab_dir, "Lab directory already removed");
    }

    let success = errors.is_empty();
    let total_duration = start_time.elapsed().as_secs();

    tracing::info!(
        lab_id = %lab_id,
        lab_name = %lab_name,
        success = success,
        total_duration_secs = total_duration,
        total_errors = errors.len(),
        "Admin clean operation completed"
    );

    Ok(DestroyResponse {
        success,
        lab_id: lab_id.to_string(),
        lab_name,
        summary,
        errors,
    })
}
