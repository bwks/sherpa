use anyhow::Result;

use virt::storage_pool::StoragePool;

use crate::data::DeviceModels;
use crate::konst::{BOOT_SERVER_NAME, SHERPA_STORAGE_POOL};
use crate::libvirt::{Qemu, get_mgmt_ip};
use crate::topology::Device;
use crate::util::{term_msg_surround, term_msg_underline};

pub fn inspect(qemu: &Qemu, lab_name: &str, lab_id: &str, devices: &[Device]) -> Result<()> {
    term_msg_surround(&format!("Sherpa Environment - {lab_name}-{lab_id}"));

    let qemu_conn = qemu.connect()?;
    let mut devices: Vec<Device> = devices.iter().map(|d| (*d).to_owned()).collect();

    let domains = qemu_conn.list_all_domains(0)?;
    let pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)?;
    devices.push(Device {
        name: BOOT_SERVER_NAME.to_owned(),
        model: DeviceModels::FlatcarLinux,
        ..Default::default()
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
            println!("Model: {}", device.model);
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
