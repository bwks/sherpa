use anyhow::{Context, Result};
use shared::data::{LabVmActionResponse, VmActionResult};
use virt::domain::Domain;

use crate::daemon::state::AppState;

/// Suspend all active VMs for a lab
pub async fn suspend_lab_vms(lab_id: &str, state: &AppState) -> Result<LabVmActionResponse> {
    let qemu = state.qemu.clone();
    let lab_id = lab_id.to_string();

    let results = tokio::task::spawn_blocking(move || -> Result<Vec<VmActionResult>> {
        let conn = qemu.connect().context("Failed to connect to libvirt")?;
        let domains = conn
            .list_all_domains(0)
            .context("Failed to list libvirt domains")?;

        let mut results = Vec::new();

        for domain in domains {
            let vm_name = match domain.get_name() {
                Ok(name) => name,
                Err(e) => {
                    tracing::warn!("Failed to get domain name: {}", e);
                    continue;
                }
            };

            if !vm_name.contains(&lab_id) {
                continue;
            }

            let result = suspend_domain(&domain, &vm_name);
            results.push(result);
        }

        Ok(results)
    })
    .await
    .context("Blocking task panicked")?
    .context("Failed to suspend lab VMs")?;

    Ok(LabVmActionResponse { results })
}

fn suspend_domain(domain: &Domain, vm_name: &str) -> VmActionResult {
    match domain.is_active() {
        Ok(true) => match domain.suspend() {
            Ok(_) => VmActionResult {
                name: vm_name.to_string(),
                success: true,
                message: "Suspended".to_string(),
            },
            Err(e) => VmActionResult {
                name: vm_name.to_string(),
                success: false,
                message: format!("Failed to suspend: {}", e),
            },
        },
        Ok(false) => VmActionResult {
            name: vm_name.to_string(),
            success: true,
            message: "Not running".to_string(),
        },
        Err(e) => VmActionResult {
            name: vm_name.to_string(),
            success: false,
            message: format!("Failed to check state: {}", e),
        },
    }
}
