use std::collections::HashMap;
use std::process::Command;

use anyhow::{Context, Result, ensure};

use shared::data::LabInfo;
use shared::konst::{BOOT_SERVER_NAME, TELNET_PORT};
use shared::util::{get_ip, term_msg_surround};
use topology::Manifest;

#[tracing::instrument(skip_all, fields(%name))]
pub fn console(name: &str, manifest: &Manifest, lab_info: &LabInfo) -> Result<()> {
    term_msg_surround(&format!("Connecting to: {name}"));

    // Node indexes start from 1, as they do for addresses; a lab holds far fewer than 255 nodes.
    let dev_id_map: HashMap<String, u8> = (1..=u8::MAX)
        .zip(&manifest.nodes)
        .map(|(index, device)| (device.name.clone(), index))
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
                .ok_or_else(|| anyhow::anyhow!("Device not found: {name}"))?;
            let device_id = dev_id_map
                .get(&device.name)
                .ok_or_else(|| anyhow::anyhow!("Device ID not found for: {}", device.name))?
                .to_owned();
            get_ip(&lab_info.loopback_network, device_id)
        }
    };

    let status = Command::new("telnet")
        .arg(device_ip.to_string())
        .arg(TELNET_PORT.to_string())
        .status()
        .context("Failed to run telnet")?;
    ensure!(status.success(), "Telnet to {device_ip} failed: {status}");

    Ok(())
}
