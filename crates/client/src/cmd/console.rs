use std::collections::HashMap;
use std::process::Command;
use std::str::FromStr;

use anyhow::{Context, Result};

use shared::data::LabInfo;
use shared::konst::{BOOT_SERVER_NAME, LAB_FILE_NAME, TELNET_PORT};
use shared::util::{get_cwd, get_ip, load_file, term_msg_surround};
use topology::Manifest;

pub fn console(name: &str, manifest: &Manifest) -> Result<()> {
    term_msg_surround(&format!("Connecting to: {name}"));

    // Load lab-info.toml to get the lab's loopback network
    let cwd = get_cwd().context("Failed to determine working directory")?;
    let lab_info_path = format!("{}/{}", cwd, LAB_FILE_NAME);
    let lab_info_content = load_file(&lab_info_path)
        .context("Failed to load lab-info.toml. Has the lab been started?")?;
    let lab_info = LabInfo::from_str(&lab_info_content).context("Failed to parse lab-info.toml")?;

    let dev_id_map: HashMap<String, u8> = manifest
        .nodes
        .iter()
        .enumerate()
        .map(|(idx, device)| (device.name.clone(), idx as u8 + 1))
        .collect();

    // Find the device in the manifest
    let device_ip = {
        if name == BOOT_SERVER_NAME {
            get_ip(&lab_info.loopback_network, 255)
        } else {
            let device = manifest
                .nodes
                .iter()
                .find(|d| d.name == *name)
                .ok_or_else(|| anyhow::anyhow!("Device not found: {}", name))?;
            let device_id = dev_id_map.get(&device.name).unwrap().to_owned(); // should never error
            get_ip(&lab_info.loopback_network, device_id)
        }
    };

    let status = Command::new("telnet")
        .arg(device_ip.to_string())
        .arg(TELNET_PORT.to_string())
        .status()?;

    if !status.success() {
        eprintln!("Telnet connection failed");
        if let Some(code) = status.code() {
            eprintln!("Exit code: {}", code);
        }
    }

    Ok(())
}
