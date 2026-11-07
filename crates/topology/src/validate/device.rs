use std::collections::HashSet;

use anyhow::{Result, bail};

use crate::Node;
use shared::konst::MANAGEMENT_NODE_HOST_OFFSETS;

/// The longest node name: one DNS label.
const NODE_NAME_MAX_LEN: usize = 63;

/// Check no two nodes share a name.
///
/// # Errors
///
/// Fails when a node name is used more than once.
pub fn check_duplicate_device(devices: &[Node]) -> Result<()> {
    let mut names = HashSet::new();
    for device in devices {
        if !names.insert(device.name.as_str()) {
            bail!(
                "Manifest - device: '{}' defined more than once",
                device.name
            );
        }
    }
    Ok(())
}

/// Check every node name is a plain name: 1 to 63 ASCII letters, digits, `-` or `_`, starting
/// with a letter or digit.
///
/// Node names become host names, container and domain names, file names in the lab folder and
/// values in the web UI, so they cannot carry path separators, quotes or markup.
///
/// # Errors
///
/// Fails on the first node whose name breaks the rule.
pub fn check_node_names(devices: &[Node]) -> Result<()> {
    for device in devices {
        let name = device.name.as_str();
        let starts_well = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
        let plain = name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !starts_well || !plain || name.len() > NODE_NAME_MAX_LEN {
            bail!(
                "Manifest - node name '{name}' is not allowed: use 1 to {NODE_NAME_MAX_LEN} \
                 letters, digits, '-' or '_', starting with a letter or digit"
            );
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
    use anyhow::Result;

    fn named(name: &str) -> Node {
        Node {
            name: name.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn plain_node_names_are_allowed() {
        let longest = "a".repeat(NODE_NAME_MAX_LEN);
        for name in ["r1", "Spine-01", "leaf_2", "9k", longest.as_str()] {
            assert!(check_node_names(&[named(name)]).is_ok(), "{name}");
        }
    }

    #[test]
    fn names_with_paths_quotes_or_markup_are_refused() {
        let too_long = "a".repeat(NODE_NAME_MAX_LEN + 1);
        for name in [
            "",
            "-r1",
            "_r1",
            "../etc",
            "r1/x",
            "r1.core",
            "r1 x",
            "x',alert(1),'",
            "<b>",
            "r\u{e9}1",
            too_long.as_str(),
        ] {
            let error = check_node_names(&[named("ok1"), named(name)])
                .expect_err(&format!("'{name}' should be refused"));
            assert!(error.to_string().contains("is not allowed"), "{error}");
        }
    }

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
