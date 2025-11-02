use std::fs;

use anyhow::Result;
use virt::storage_pool::StoragePool;
use virt::sys::VIR_DOMAIN_UNDEFINE_NVRAM;

use crate::konst::{SHERPA_STORAGE_POOL, SHERPA_STORAGE_POOL_PATH, TEMP_DIR};
use crate::libvirt::{Qemu, delete_disk};
use crate::util::{dir_exists, file_exists, term_msg_surround};

pub fn destroy(qemu: &Qemu, lab_name: &str, lab_id: &str) -> Result<()> {
    term_msg_surround(&format!("Destroying environment - {lab_name}-{lab_id}"));

    let qemu_conn = qemu.connect()?;
    let domains = qemu_conn.list_all_domains(0)?;
    let storage_pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)?;
    let pool_disks = storage_pool.list_volumes()?;

    for domain in domains {
        let vm_name = domain.get_name()?;
        if vm_name.contains(lab_id) && domain.is_active()? {
            // EUFI domains will have an NVRAM file that must be deleted.
            let nvram_flag = VIR_DOMAIN_UNDEFINE_NVRAM;
            domain.undefine_flags(nvram_flag)?;
            domain.destroy()?;
            println!("Destroyed VM: {vm_name}");

            // Destroy disks
            let domain_disks: Vec<&String> = pool_disks
                .iter()
                .filter(|d| d.starts_with(&vm_name))
                .collect();

            for disk in domain_disks {
                if file_exists(&format!("{SHERPA_STORAGE_POOL_PATH}/{disk}")) {
                    delete_disk(&qemu_conn, disk)?;
                    println!("Deleted disk: {disk}");
                }
            }
        }
    }
    if dir_exists(TEMP_DIR) {
        fs::remove_dir_all(TEMP_DIR)?;
        println!("Deleted directory: {TEMP_DIR}");
    }
    Ok(())
}
