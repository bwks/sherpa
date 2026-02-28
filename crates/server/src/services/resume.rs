use anyhow::{Context, Result};
use shared::data::{LabVmActionResponse, VmActionResult};
use virt::domain::Domain;
use virt::sys::{VIR_DOMAIN_PAUSED, VIR_DOMAIN_RUNNING};

use crate::daemon::state::AppState;

/// Resume all paused VMs for a lab
pub async fn resume_lab_vms(lab_id: &str, state: &AppState) -> Result<LabVmActionResponse> {
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

            let result = resume_domain(&domain, &vm_name);
            results.push(result);
        }

        Ok(results)
    })
    .await
    .context("Blocking task panicked")?
    .context("Failed to resume lab VMs")?;

    Ok(LabVmActionResponse { results })
}

fn resume_domain(domain: &Domain, vm_name: &str) -> VmActionResult {
    match domain.get_state() {
        Ok((state, _reason)) => {
            if state == VIR_DOMAIN_PAUSED {
                match domain.resume() {
                    Ok(_) => VmActionResult {
                        name: vm_name.to_string(),
                        success: true,
                        message: "Resumed".to_string(),
                    },
                    Err(e) => VmActionResult {
                        name: vm_name.to_string(),
                        success: false,
                        message: format!("Failed to resume: {}", e),
                    },
                }
            } else if state == VIR_DOMAIN_RUNNING {
                VmActionResult {
                    name: vm_name.to_string(),
                    success: true,
                    message: "Already running".to_string(),
                }
            } else {
                VmActionResult {
                    name: vm_name.to_string(),
                    success: false,
                    message: format!("Not paused (state: {})", state),
                }
            }
        }
        Err(e) => VmActionResult {
            name: vm_name.to_string(),
            success: false,
            message: format!("Failed to get state: {}", e),
        },
    }
}
