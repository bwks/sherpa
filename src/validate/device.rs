use anyhow::{bail, Result};

use crate::topology::Device;

// Check duplicate device definitions
pub fn check_duplicate_device(devices: &Vec<Device>) -> Result<()> {
    let mut devs: Vec<String> = vec![];

    for device in devices {
        if devs.contains(&device.name) {
            bail!(
                "Manifest - device: '{}' defined more than once",
                &device.name
            );
        } else {
            devs.push(device.name.clone())
        }
    }
    Ok(())
}
