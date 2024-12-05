use anyhow::Result;

use virt::storage_pool::StoragePool;

use crate::core::konst::{BOOT_SERVER_NAME, SHERPA_MANIFEST_FILE, SHERPA_STORAGE_POOL};
use crate::data::DeviceModels;
use crate::libvirt::{get_mgmt_ip, Qemu};
use crate::topology::{Device, Manifest};
use crate::util::{get_id, term_msg_surround, term_msg_underline};

pub fn inspect(qemu: &Qemu) -> Result<()> {
    let lab_id = get_id()?;

    let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
    let lab_name = manifest.name.clone();

    term_msg_surround(&format!("Sherpa Environment - {lab_name}-{lab_id}"));

    let qemu_conn = qemu.connect()?;

    let domains = qemu_conn.list_all_domains(0)?;
    let pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)?;
    let mut devices = manifest.devices;
    devices.push(Device {
        name: BOOT_SERVER_NAME.to_owned(),
        device_model: DeviceModels::FlatcarLinux,
    });
    let mut inactive_devices = vec![];
    for device in devices {
        let device_name = format!("{}-{}-{}", device.name, lab_name, lab_id);
        if let Some(domain) = domains
            .iter()
            .find(|d| d.get_name().unwrap_or_default() == device_name)
        {
            term_msg_underline(&device.name);
            println!("Domain: {}", device_name);
            println!("Model: {}", device.device_model);
            println!("Active: {:#?}", domain.is_active()?);
            if let Some(vm_ip) = get_mgmt_ip(&qemu_conn, &device_name)? {
                println!("Mgmt IP: {vm_ip}");
            }
            for volume in pool.list_volumes()? {
                if volume.contains(&device_name) {
                    println!("Disk: {volume}");
                }
            }
        } else {
            inactive_devices.push(device.name);
        }
    }

    if !inactive_devices.is_empty() {
        term_msg_underline("Inactive Devices");
        for device in &inactive_devices {
            println!("{device}")
        }
    }
    Ok(())
}
