use std::collections::HashSet;

use anyhow::{Result, bail};

use crate::{Node, TopologyError};
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
            bail!(TopologyError::DuplicateNode {
                node: device.name.clone(),
            });
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
        bail!(TopologyError::TooManyNodes {
            count: devices.len(),
            max,
        });
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
        let error = check_duplicate_device(&devices).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::DuplicateNode {
                node: "router1".to_owned(),
            })
        );
    }

    #[test]
    fn node_naming_004_a_manifest_with_235_nodes_is_refused() {
        let devices = vec![Node::default(); 235];
        let error = check_device_count(&devices).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::TooManyNodes {
                count: 235,
                max: 234,
            })
        );
    }
}
