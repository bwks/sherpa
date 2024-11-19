use anyhow::Result;
use virt::sys::{VIR_DOMAIN_PAUSED, VIR_DOMAIN_RUNNING};

use crate::libvirt::Qemu;
use crate::util::{get_id, term_msg_surround};

pub fn resume(qemu: &Qemu) -> Result<()> {
    term_msg_surround("Resuming environment");

    let lab_id = get_id()?;

    let qemu_conn = qemu.connect()?;

    let domains = qemu_conn.list_all_domains(0)?;

    for domain in domains {
        let vm_name = domain.get_name()?;
        if vm_name.contains(&lab_id) {
            match domain.get_state() {
                Ok((state, _reason)) => {
                    if state == VIR_DOMAIN_PAUSED {
                        domain.resume()?;
                        println!("Resumed: {vm_name}");
                    } else if state == VIR_DOMAIN_RUNNING {
                        println!("Virtual machine already running: {vm_name}");
                    } else {
                        println!("Virtual machine not paused (state: {}): {}", state, vm_name);
                    }
                }
                Err(e) => anyhow::bail!("Failed to get state for {vm_name}: {e}"),
            }
        }
    }
    Ok(())
}
