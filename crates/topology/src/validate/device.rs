use std::collections::HashSet;

use anyhow::{Result, bail};

use crate::Node;
use shared::konst::MANAGEMENT_NODE_HOST_OFFSETS;

/// Check no two nodes share a name.
///
/// # Errors
///
/// Fails when a node name is used more than once.
pub fn check_duplicate_device(devices: &[Node]) -> Result<()> {
    let mut names = HashSet::new();
    for device in devices {
        if !names.insert(device.name.as_str()) {
            bail!("Manifest - node: '{}' defined more than once", device.name);
        }
    }
    Ok(())
}

/// Check the manifest has no more nodes than the lab management node block has addresses.
///
/// # Errors
///
/// Fails when there are more nodes than addresses.
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

    #[test]
    fn node_naming_003_two_nodes_with_one_name_are_refused_naming_it() {
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
                .contains("Manifest - node: 'router1' defined more than once")
        );
    }

    #[test]
    fn node_naming_004_a_manifest_with_235_nodes_is_refused() {
        let devices = vec![Node::default(); 235];
        let error = check_device_count(&devices).unwrap_err().to_string();
        assert!(error.contains("235 nodes defined"));
        assert!(error.contains("at most 234 nodes"));
    }
}
