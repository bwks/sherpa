use std::process::Command;

use anyhow::Result;

use crate::core::konst::{BOOT_SERVER_NAME, TELNET_PORT};
use crate::topology::Manifest;
use crate::util::{get_ip, term_msg_surround};

pub fn console(name: &str) -> Result<()> {
    term_msg_surround(&format!("Connecting to: {name}"));

    let manifest = Manifest::load_file()?;

    // Find the device in the manifest
    let device_ip = {
        if name == BOOT_SERVER_NAME {
            get_ip(255)
        } else {
            let device = manifest
                .devices
                .iter()
                .find(|d| d.name == *name)
                .ok_or_else(|| anyhow::anyhow!("Device not found: {}", name))?;
            get_ip(device.id)
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
