use std::fs;

use anyhow::Result;
use virt::sys::VIR_DOMAIN_UNDEFINE_NVRAM;

use crate::core::konst::{SHERPA_MANIFEST_FILE, SHERPA_STORAGE_POOL_PATH, TEMP_DIR};
use crate::libvirt::{delete_disk, Qemu};
use crate::topology::Manifest;
use crate::util::{dir_exists, file_exists, get_id, term_msg_surround};

pub fn destroy(qemu: &Qemu) -> Result<()> {
    let lab_id = get_id()?;
    let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
    let lab_name = manifest.name;
    term_msg_surround(&format!("Destroying environment - {lab_name}-{lab_id}"));

    let qemu_conn = qemu.connect()?;
    let domains = qemu_conn.list_all_domains(0)?;

    for domain in domains {
        let vm_name = domain.get_name()?;
        if vm_name.contains(&lab_id) && domain.is_active()? {
            // EUFI domains will have an NVRAM file that must be deleted.
            let nvram_flag = VIR_DOMAIN_UNDEFINE_NVRAM;
            domain.undefine_flags(nvram_flag)?;
            domain.destroy()?;
            println!("Destroyed VM: {vm_name}");

            // HDD
            let hdd_name = format!("{vm_name}.qcow2");
            delete_disk(&qemu_conn, &hdd_name)?;
            println!("Deleted HDD: {hdd_name}");

            // ISO
            let iso_name = format!("{vm_name}.iso");
            if file_exists(&format!("{SHERPA_STORAGE_POOL_PATH}/{iso_name}")) {
                delete_disk(&qemu_conn, &iso_name)?;
                println!("Deleted ISO: {iso_name}");
            }

            // Ignition
            let ign_name = format!("{vm_name}.ign");
            if file_exists(&format!("{SHERPA_STORAGE_POOL_PATH}/{ign_name}")) {
                delete_disk(&qemu_conn, &ign_name)?;
                println!("Deleted Ignition: {ign_name}");
            }

            // Disk Image
            let disk_name = format!("{vm_name}.img");
            if file_exists(&format!("{SHERPA_STORAGE_POOL_PATH}/{disk_name}")) {
                delete_disk(&qemu_conn, &disk_name)?;
                println!("Deleted Disk: {disk_name}");
            }
        }
    }
    if dir_exists(TEMP_DIR) {
        fs::remove_dir_all(TEMP_DIR)?;
        println!("Deleted directory: {TEMP_DIR}");
    }
    Ok(())
}
