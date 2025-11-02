use std::collections::HashMap;
use std::process::Command;

use anyhow::Result;

use crate::konst::{BOOT_SERVER_NAME, TELNET_PORT};
use crate::topology::Manifest;
use crate::util::{get_ip, term_msg_surround};

pub fn console(name: &str, manifest: &Manifest) -> Result<()> {
    term_msg_surround(&format!("Connecting to: {name}"));

    let dev_id_map: HashMap<String, u8> = manifest
        .devices
        .iter()
        .enumerate()
        .map(|(idx, device)| (device.name.clone(), idx as u8 + 1))
        .collect();

    // Find the device in the manifes
    let device_ip = {
        if name == BOOT_SERVER_NAME {
            get_ip(255)
        } else {
            let device = manifest
                .devices
                .iter()
                .find(|d| d.name == *name)
                .ok_or_else(|| anyhow::anyhow!("Device not found: {}", name))?;
            let device_id = dev_id_map.get(&device.name).unwrap().to_owned(); // should never error
            get_ip(device_id)
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
