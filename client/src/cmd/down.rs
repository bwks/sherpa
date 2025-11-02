use anyhow::Result;

use crate::libvirt::Qemu;
use crate::util::term_msg_surround;

pub fn down(qemu: &Qemu, lab_id: &str) -> Result<()> {
    term_msg_surround("Suspending environment");
    let qemu_conn = qemu.connect()?;

    let domains = qemu_conn.list_all_domains(0)?;

    for domain in domains {
        let vm_name = domain.get_name()?;
        if vm_name.contains(lab_id) {
            if domain.is_active()? {
                domain.suspend()?;
                println!("Suspended: {vm_name}");
            } else {
                println!("Virtual machine not running: {vm_name}");
            }
        }
    }
    Ok(())
}
