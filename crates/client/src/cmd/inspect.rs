use std::str::FromStr;

use anyhow::{Result, bail};
use serde_derive::{Deserialize, Serialize};
use virt::storage_pool::StoragePool;

use libvirt::Qemu;
use shared::data::{Config, LabInfo, NodeModel};
use shared::konst::{LAB_FILE_NAME, SHERPA_BASE_DIR, SHERPA_LABS_DIR, SHERPA_STORAGE_POOL};
use shared::util::{get_dhcp_leases, load_file, term_msg_surround, term_msg_underline};
use topology::Node;

#[derive(Debug, Serialize, Deserialize)]
struct InpsectDevice {
    name: String,
    model: NodeModel,
    active: bool,
    mgmt_ip: String,
    disks: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct InspectData {
    lab_name: String,
    lab_id: String,
    devices: Vec<InpsectDevice>,
}

pub async fn inspect(
    qemu: &Qemu,
    lab_name: &str,
    lab_id: &str,
    config: &Config,
    devices: &[Node],
) -> Result<()> {
    term_msg_surround(&format!("Sherpa Environment - {lab_name}-{lab_id}"));
    let lab_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_LABS_DIR}/{lab_id}");

    term_msg_underline("Lab Info");

    let lab_file = match load_file(&format!("{lab_dir}/{LAB_FILE_NAME}")) {
        Ok(f) => f,
        Err(_) => {
            bail!("Unable to load lab file. Is the lab running?")
        }
    };
    let lab_info = LabInfo::from_str(&lab_file)?;
    println!("{}", lab_info);

    let qemu_conn = qemu.connect()?;
    let devices: Vec<Node> = devices.iter().map(|d| (*d).to_owned()).collect();

    let domains = qemu_conn.list_all_domains(0)?;
    let pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)?;

    let mut inspect_data = InspectData {
        lab_name: lab_name.to_string(),
        lab_id: lab_id.to_string(),
        devices: vec![],
    };
    let mut inspect_devices: Vec<InpsectDevice> = vec![];

    let leases = get_dhcp_leases(config).await?;
    let mut inactive_devices = vec![];
    for device in devices {
        let mut device_data = InpsectDevice {
            name: device.name.clone(),
            model: device.model.clone(),
            active: false,
            mgmt_ip: "".to_string(),
            disks: vec![],
        };
        let device_name = format!("{}-{}", device.name, lab_id);

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
                device_data.mgmt_ip = vm_ip;
                device_data.active = true;
            }
            let mut device_disks = vec![];
            for volume in pool.list_volumes()? {
                if volume.contains(&device_name) {
                    println!("Disk: {volume}");
                    device_disks.push(volume)
                }
            }
            device_data.disks = device_disks;
            inspect_devices.push(device_data)
        } else {
            inactive_devices.push(device.name);
        }
    }

    inspect_data.devices = inspect_devices;

    println!("{}", serde_json::to_string_pretty(&inspect_data)?);

    if !inactive_devices.is_empty() {
        term_msg_underline("inactive devices");
        for device in &inactive_devices {
            println!("{device}")
        }
    }
    Ok(())
}
