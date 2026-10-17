use anyhow::{Result, bail};

use shared::konst::MANAGEMENT_NODE_HOST_OFFSETS;
use topology::Node;

// Check duplicate device definitions
pub fn check_duplicate_device(devices: &Vec<Node>) -> Result<()> {
    let mut devs: Vec<String> = vec![];

    for device in devices {
        if devs.contains(&device.name) {
            bail!(
                "Manifest - device: '{}' defined more than once",
                device.name
            );
        } else {
            devs.push(device.name.clone())
        }
    }
    Ok(())
}

/// Check the manifest has no more nodes than the lab management node block has addresses.
pub fn check_device_count(devices: &[Node]) -> Result<()> {
    let max = MANAGEMENT_NODE_HOST_OFFSETS.clone().count();
    if devices.len() > max {
        bail!(
            "Manifest - {} nodes defined, but a lab supports at most {max} nodes",
            devices.len()
        );
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
            Node {
                name: "router1".to_string(),
                ..Default::default()
            },
            Node {
                name: "router2".to_string(),
                ..Default::default()
            },
            Node {
                name: "switch1".to_string(),
                ..Default::default()
            },
        ];
        check_duplicate_device(&devices)
    }

    #[test]
    fn test_check_duplicate_device_with_duplicates() {
        let devices = vec![
            Node {
                name: "router1".to_string(),
                ..Default::default()
            },
            Node {
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
        let devices = vec![Node {
            name: "router1".to_string(),
            ..Default::default()
        }];
        check_duplicate_device(&devices)
    }

    #[test]
    fn test_check_device_count_allows_a_full_node_block() -> Result<()> {
        let devices = vec![Node::default(); 234];
        check_device_count(&devices)
    }

    #[test]
    fn test_check_device_count_rejects_more_nodes_than_addresses() {
        let devices = vec![Node::default(); 235];
        let error = check_device_count(&devices).unwrap_err().to_string();
        assert!(error.contains("235 nodes defined"));
        assert!(error.contains("at most 234 nodes"));
    }
}
