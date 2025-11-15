use anyhow::Result;

use virt::storage_pool::StoragePool;

use data::Config;
use konst::SHERPA_STORAGE_POOL;
use libvirt::Qemu;
use topology::Device;
use util::{get_dhcp_leases, term_msg_surround, term_msg_underline};

pub async fn inspect(
    qemu: &Qemu,
    lab_name: &str,
    lab_id: &str,
    config: &Config,
    devices: &[Device],
) -> Result<()> {
    term_msg_surround(&format!("Sherpa Environment - {lab_name}-{lab_id}"));

    let qemu_conn = qemu.connect()?;
    let devices: Vec<Device> = devices.iter().map(|d| (*d).to_owned()).collect();

    let domains = qemu_conn.list_all_domains(0)?;
    let pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)?;

    let leases = get_dhcp_leases(&config).await?;
    let mut inactive_devices = vec![];
    for device in devices {
        let device_name = format!("{}-{}-{}", device.name, lab_name, lab_id);

        if let Some(domain) = domains
            .iter()
            .find(|d| d.get_name().unwrap_or_default() == device_name)
        {
            let vm_ip = if let Some(vm_ip) = leases.iter().find(|d| d.hostname == device.name) {
                vm_ip.ipv4_address.clone()
            } else {
                "".to_owned()
            };
            term_msg_underline(&device.name);
            println!("Domain: {}", device_name);
            println!("Model: {}", device.model);
            println!("Active: {:#?}", domain.is_active()?);
            if !vm_ip.is_empty() {
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
