use anyhow::{Result, bail};

use topology::Device;

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
#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn test_check_duplicate_device_no_duplicates() -> Result<()> {
        let devices = vec![
            Device {
                name: "router1".to_string(),
                ..Default::default()
            },
            Device {
                name: "router2".to_string(),
                ..Default::default()
            },
            Device {
                name: "switch1".to_string(),
                ..Default::default()
            },
        ];
        check_duplicate_device(&devices)
    }

    #[test]
    fn test_check_duplicate_device_with_duplicates() {
        let devices = vec![
            Device {
                name: "router1".to_string(),
                ..Default::default()
            },
            Device {
                name: "router1".to_string(),
                ..Default::default()
            },
        ];
        let result = check_duplicate_device(&devices);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Manifest - device: 'router1' defined more than once")
        );
    }

    #[test]
    fn test_check_duplicate_device_empty() -> Result<()> {
        let devices = vec![];
        check_duplicate_device(&devices)
    }

    #[test]
    fn test_check_duplicate_device_single() -> Result<()> {
        let devices = vec![Device {
            name: "router1".to_string(),
            ..Default::default()
        }];
        check_duplicate_device(&devices)
    }
}
