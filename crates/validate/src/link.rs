use std::collections::{HashMap, HashSet};

use anyhow::{bail, Result};

use shared::data::NodeModel;
use topology::{BridgeDetailed, LinkDetailed, Node};

/// Checks if any links or bridges use the management interface (index 0) on a node.
/// Returns an error if a link or bridge attempts to use the management interface.
/// This validation only applies to nodes without dedicated management interfaces.
pub fn check_mgmt_usage(
    device_name: &str,
    mgmt_interface_index: u8,
    links: &Vec<LinkDetailed>,
    bridges: &[BridgeDetailed],
) -> Result<()> {
    // Check point-to-point links
    for link in links {
        let (device, interface_idx, interface_name) = // no-fmt
        if device_name == link.node_a {
            (device_name, link.int_a_idx, link.int_a.clone())
        } else if device_name == link.node_b {
            (device_name, link.int_b_idx, link.int_b.clone())
        } else {
            continue; // this will skip to the next loop if device not matched in link
        };
        if device_name == device && mgmt_interface_index == interface_idx {
            bail!(
                "Manifest link - '{device}' interface '{interface_name}' overlaps with management interface",
            );
        }
    }

    // Check bridge connections
    for bridge in bridges {
        for bridge_link in &bridge.links {
            if bridge_link.node_name == device_name
                && bridge_link.interface_index == mgmt_interface_index
            {
                bail!(
                    "Manifest bridge '{}' - device '{}' interface '{}' overlaps with management interface",
                    bridge.manifest_name,
                    device_name,
                    bridge_link.interface_name
                );
            }
        }
    }

    Ok(())
}

/// Check for duplicate interface usage across device links and bridges.
/// Each interface on a device can only be used once (either in a link OR a bridge).
pub fn check_duplicate_interface_link(
    links: &Vec<LinkDetailed>,
    bridges: &[BridgeDetailed],
) -> Result<()> {
    let mut device_int_map: HashMap<String, Vec<u8>> = HashMap::new();

    // Check point-to-point links
    for link in links {
        check_device_interface(&link.node_a, link.int_a_idx, &mut device_int_map, "link")?;
        check_device_interface(&link.node_b, link.int_b_idx, &mut device_int_map, "link")?;
    }

    // Check bridge connections
    for bridge in bridges {
        for bridge_link in &bridge.links {
            check_device_interface(
                &bridge_link.node_name,
                bridge_link.interface_index,
                &mut device_int_map,
                &format!("bridge '{}'", bridge.manifest_name),
            )?;
        }
    }

    Ok(())
}
/// Helper function for `check_duplicate_interface_link` function
fn check_device_interface(
    device: &str,
    interface: u8,
    device_int_map: &mut HashMap<String, Vec<u8>>,
    connection_type: &str,
) -> Result<()> {
    match device_int_map.get_mut(device) {
        Some(interfaces) => {
            if interfaces.contains(&interface) {
                bail!(
                    "Manifest {} - device '{}' interface index '{}' is already in use",
                    connection_type,
                    device,
                    interface
                );
            }
            interfaces.push(interface);
        }
        None => {
            device_int_map.insert(device.to_string(), vec![interface]);
        }
    }
    Ok(())
}

/// Check devices defined in links are defined as top level devices
pub fn check_link_device(devices: &[Node], links: &Vec<LinkDetailed>) -> Result<()> {
    let unique_devices: Vec<String> = devices.iter().map(|d| d.name.clone()).collect();
    let mut unique_device_link: HashSet<String> = HashSet::new();
    for link in links {
        unique_device_link.insert(link.node_a.clone());
        unique_device_link.insert(link.node_b.clone());
    }
    for device in &unique_device_link {
        if !unique_devices.contains(device) {
            bail!("Manifest link - '{device}' defined in links, not defined in devices");
        }
    }
    Ok(())
}

/// Check devices defined in bridges are defined as top level devices
pub fn check_bridge_device(devices: &[Node], bridges: &[BridgeDetailed]) -> Result<()> {
    let unique_devices: Vec<String> = devices.iter().map(|d| d.name.clone()).collect();
    let mut unique_device_bridge: HashSet<String> = HashSet::new();

    for bridge in bridges {
        for bridge_link in &bridge.links {
            unique_device_bridge.insert(bridge_link.node_name.clone());
        }
    }

    for device in &unique_device_bridge {
        if !unique_devices.contains(device) {
            bail!(
                "Manifest bridge - device '{}' defined in bridges, not defined in devices",
                device
            );
        }
    }
    Ok(())
}

/// Check interface index bounds for both links and bridges.
/// Interfaces defined in connections should not be:
/// - the management interface (if dedicated_management_interface is true)
/// - reserved interfaces
/// - greater than the maximum interface index calculated from data_interface_count
pub fn check_interface_bounds(
    device_name: &str,
    device_model: &NodeModel,
    data_interface_count: u8,
    reserved_interface_count: u8,
    dedicated_management_interface: bool,
    links: &Vec<LinkDetailed>,
    bridges: &[BridgeDetailed],
) -> Result<()> {
    // Calculate the valid interface range
    // Management is always at index 0
    // Reserved interfaces are at indices 1 to reserved_interface_count
    // Data interfaces start at (1 + reserved_interface_count)
    let first_data_interface_idx = 1 + reserved_interface_count;

    // Maximum interface index = first_data_interface_idx + data_interface_count - 1
    // For example: data_interface_count=52, reserved=0 -> first_data=1, max=52
    let max_interface_idx = first_data_interface_idx + data_interface_count - 1;

    // Check point-to-point links
    for link in links {
        let (device, interface_idx, interface_name) = // no-fmt
        if device_name == link.node_a {
            (device_name, link.int_a_idx, link.int_a.clone())
        } else if device_name == link.node_b {
            (device_name, link.int_b_idx, link.int_b.clone())
        } else {
            continue; // this will skip to the next loop if device not matched in link
        };

        // Check if interface is in valid range
        if interface_idx == 0 {
            if dedicated_management_interface {
                bail!(
                    "Manifest link - device '{device}' interface '{interface_name}' is the management interface and cannot be used in links"
                )
            }
            // If not dedicated, index 0 can be used for data, so it's valid
        } else if interface_idx > 0 && interface_idx < first_data_interface_idx {
            bail!(
                "Manifest link - device '{device}' interface index '{interface_name}' is a reserved interface (reserved_count: {reserved_interface_count}) and cannot be used in links"
            )
        } else if interface_idx > max_interface_idx {
            bail!(
                "Manifest link - device '{device}' interface index '{interface_name}' exceeds the maximum interface index '{max_interface_idx}' for device model '{device_model}' (data_interface_count: {data_interface_count}, reserved: {reserved_interface_count})"
            )
        }
    }

    // Check bridge connections
    for bridge in bridges {
        for bridge_link in &bridge.links {
            if bridge_link.node_name != device_name {
                continue;
            }

            let interface_idx = bridge_link.interface_index;
            let interface_name = &bridge_link.interface_name;

            // Check if interface is in valid range
            if interface_idx == 0 {
                if dedicated_management_interface {
                    bail!(
                        "Manifest bridge '{}' - device '{}' interface '{}' is the management interface and cannot be used in bridges",
                        bridge.manifest_name,
                        device_name,
                        interface_name
                    )
                }
                // If not dedicated, index 0 can be used for data, so it's valid
            } else if interface_idx > 0 && interface_idx < first_data_interface_idx {
                bail!(
                    "Manifest bridge '{}' - device '{}' interface '{}' is a reserved interface (reserved_count: {}) and cannot be used in bridges",
                    bridge.manifest_name,
                    device_name,
                    interface_name,
                    reserved_interface_count
                )
            } else if interface_idx > max_interface_idx {
                bail!(
                    "Manifest bridge '{}' - device '{}' interface '{}' exceeds the maximum interface index '{}' for device model '{}' (data_interface_count: {}, reserved: {})",
                    bridge.manifest_name,
                    device_name,
                    interface_name,
                    max_interface_idx,
                    device_model,
                    data_interface_count,
                    reserved_interface_count
                )
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use topology::BridgeLinkDetailed;

    /// Helper to create a test link
    fn create_link(node_a: &str, int_a: u8, node_b: &str, int_b: u8) -> LinkDetailed {
        LinkDetailed {
            node_a: node_a.to_string(),
            node_a_idx: 0,
            node_a_model: NodeModel::RockyLinux,
            int_a: "test".to_string(),
            int_a_idx: int_a,
            node_b: node_b.to_string(),
            node_b_idx: 1,
            node_b_model: NodeModel::RockyLinux,
            int_b: "test".to_string(),
            int_b_idx: int_b,
            link_idx: 0,
        }
    }

    #[test]
    fn test_interface_bounds_valid_data_interface() -> Result<()> {
        // Rocky Linux with 52 data interfaces
        let links = vec![create_link("rocky1", 52, "rocky2", 1)];
        let bridges = vec![];

        check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            52,   // data_interface_count
            0,    // reserved_interface_count
            true, // dedicated_management_interface
            &links,
            &bridges,
        )
    }

    #[test]
    fn test_interface_bounds_exceeds_maximum() {
        // Rocky Linux with 52 data interfaces, trying to use interface 53
        let links = vec![create_link("rocky1", 53, "rocky2", 1)];
        let bridges = vec![];

        let result = check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            52,
            0,
            true,
            &links,
            &bridges,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("exceeds the maximum interface index"));
        assert!(err_msg.contains("52")); // max interface index
    }

    #[test]
    fn test_interface_bounds_management_interface_dedicated() {
        // Rocky Linux with dedicated management, trying to use interface 0
        let links = vec![create_link("rocky1", 0, "rocky2", 1)];
        let bridges = vec![];

        let result = check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            52,
            0,
            true, // dedicated_management_interface = true
            &links,
            &bridges,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("is the management interface and cannot be used in links"));
    }

    #[test]
    fn test_interface_bounds_management_interface_not_dedicated() -> Result<()> {
        // CentOS with non-dedicated management, using interface 0 is OK
        let links = vec![create_link("centos1", 0, "centos2", 0)];
        let bridges = vec![];

        check_interface_bounds(
            "centos1",
            &NodeModel::CentosLinux,
            1,
            0,
            false, // dedicated_management_interface = false
            &links,
            &bridges,
        )
    }

    #[test]
    fn test_interface_bounds_reserved_interface() {
        // Cisco IOS XRv9000 with 2 reserved interfaces
        let links = vec![create_link("xr1", 2, "xr2", 3)];
        let bridges = vec![];

        let result = check_interface_bounds(
            "xr1",
            &NodeModel::CiscoIosxrv9000,
            31,
            2, // reserved_interface_count = 2 (indices 1 and 2)
            true,
            &links,
            &bridges,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("reserved interface"));
    }

    #[test]
    fn test_interface_bounds_first_data_interface_after_reserved() -> Result<()> {
        // Cisco IOS XRv9000: interface 3 is first data interface
        let links = vec![create_link("xr1", 3, "xr2", 3)];
        let bridges = vec![];

        check_interface_bounds(
            "xr1",
            &NodeModel::CiscoIosxrv9000,
            31,
            2, // reserved_interface_count = 2
            true,
            &links,
            &bridges,
        )
    }

    #[test]
    fn test_interface_bounds_max_with_reserved() -> Result<()> {
        // Cisco IOS XRv9000: max data interface index = 3 + 31 - 1 = 33
        let links = vec![create_link("xr1", 33, "xr2", 3)];
        let bridges = vec![];

        check_interface_bounds(
            "xr1",
            &NodeModel::CiscoIosxrv9000,
            31,
            2,
            true,
            &links,
            &bridges,
        )
    }

    #[test]
    fn test_interface_bounds_link_not_involving_device() -> Result<()> {
        // Link doesn't involve the device being checked
        let links = vec![create_link("rocky1", 100, "rocky2", 100)];
        let bridges = vec![];

        // Check device "rocky3" which is not in the link
        check_interface_bounds(
            "rocky3",
            &NodeModel::RockyLinux,
            52,
            0,
            true,
            &links,
            &bridges,
        )
    }

    #[test]
    fn test_interface_bounds_empty_links() -> Result<()> {
        let links = vec![];
        let bridges = vec![];

        check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            52,
            0,
            true,
            &links,
            &bridges,
        )
    }

    #[test]
    fn test_interface_bounds_minimum_data_interface() -> Result<()> {
        // First data interface (index 1) for Rocky Linux
        let links = vec![create_link("rocky1", 1, "rocky2", 1)];
        let bridges = vec![];

        check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            52,
            0,
            true,
            &links,
            &bridges,
        )
    }

    #[test]
    fn test_interface_bounds_single_interface_device() -> Result<()> {
        // Device with only 1 data interface (data_interface_count=1)
        // Valid indices: 0 (mgmt), 1 (data)
        let links = vec![create_link("rocky1", 1, "rocky2", 1)];
        let bridges = vec![];

        check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            1, // data_interface_count = 1
            0,
            true,
            &links,
            &bridges,
        )
    }

    #[test]
    fn test_interface_bounds_single_interface_device_exceeds() {
        // Device with only 1 data interface, trying to use interface 2
        let links = vec![create_link("rocky1", 2, "rocky2", 1)];
        let bridges = vec![];

        let result = check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            1,
            0,
            true,
            &links,
            &bridges,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_check_mgmt_usage_blocks_eth0() {
        // Test: eth0 (index 0) should be blocked as management interface
        let links = vec![create_link("node1", 0, "node2", 1)];
        let bridges = vec![];

        let result = check_mgmt_usage("node1", 0, &links, &bridges);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("management interface"));
    }

    #[test]
    fn test_check_mgmt_usage_allows_eth1() -> Result<()> {
        // Test: eth1 (index 1) should be allowed for data links
        let links = vec![create_link("node1", 1, "node2", 1)];
        let bridges = vec![];

        check_mgmt_usage("node1", 0, &links, &bridges)
    }

    #[test]
    fn test_check_mgmt_usage_allows_higher_interfaces() -> Result<()> {
        // Test: eth2, eth3, etc. should all be allowed
        let links = vec![create_link("node1", 2, "node2", 3)];
        let bridges = vec![];

        check_mgmt_usage("node1", 0, &links, &bridges)
    }

    /// Helper to create a test bridge with connections
    fn create_bridge(bridge_name: &str, connections: Vec<(&str, u8)>) -> BridgeDetailed {
        BridgeDetailed {
            manifest_name: bridge_name.to_string(), // Use bridge_name as manifest_name for testing
            bridge_name: bridge_name.to_string(),
            libvirt_name: format!("test_{}", bridge_name),
            index: 0,
            links: connections
                .into_iter()
                .map(|(node_name, interface_index)| BridgeLinkDetailed {
                    node_name: node_name.to_string(),
                    node_model: NodeModel::RockyLinux,
                    interface_name: format!("eth{}", interface_index),
                    interface_index,
                })
                .collect(),
        }
    }

    // ============================================================================
    // Bridge-specific tests
    // ============================================================================

    #[test]
    fn test_bridge_interface_bounds_valid() -> Result<()> {
        // Bridge using valid data interfaces
        let links = vec![];
        let bridges = vec![create_bridge("br1", vec![("rocky1", 10), ("rocky2", 15)])];

        check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            52,   // data_interface_count
            0,    // reserved_interface_count
            true, // dedicated_management_interface
            &links,
            &bridges,
        )
    }

    #[test]
    fn test_bridge_interface_bounds_exceeds_max() {
        // Bridge exceeding max interface for a device
        let links = vec![];
        let bridges = vec![create_bridge("br1", vec![("rocky1", 100)])];

        let result = check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            52,
            0,
            true,
            &links,
            &bridges,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("bridge 'br1'"));
        assert!(err_msg.contains("exceeds the maximum interface index"));
    }

    #[test]
    fn test_bridge_mgmt_usage_blocked() {
        // Bridge attempting to use management interface (eth0)
        let links = vec![];
        let bridges = vec![create_bridge("br1", vec![("rocky1", 0), ("rocky2", 5)])];

        let result = check_mgmt_usage("rocky1", 0, &links, &bridges);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("bridge 'br1'"));
        assert!(err_msg.contains("management interface"));
    }

    #[test]
    fn test_bridge_reserved_interface_blocked() {
        // Bridge attempting to use reserved interface
        let links = vec![];
        let bridges = vec![create_bridge("br1", vec![("rocky1", 1)])];

        let result = check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            52,
            5, // reserved_interface_count - indices 1-5 are reserved
            true,
            &links,
            &bridges,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("bridge 'br1'"));
        assert!(err_msg.contains("reserved interface"));
    }

    #[test]
    fn test_duplicate_interface_link_and_bridge() {
        // Same interface used in both a link AND a bridge (should fail)
        let links = vec![create_link("rocky1", 5, "rocky2", 10)];
        let bridges = vec![create_bridge("br1", vec![("rocky1", 5), ("rocky3", 15)])];

        let result = check_duplicate_interface_link(&links, &bridges);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("rocky1"));
        assert!(err_msg.contains("interface index '5'"));
        assert!(err_msg.contains("already in use"));
    }

    #[test]
    fn test_duplicate_interface_multiple_bridges() {
        // Same interface used in multiple bridges (should fail)
        let links = vec![];
        let bridges = vec![
            create_bridge("br1", vec![("rocky1", 10), ("rocky2", 15)]),
            create_bridge("br2", vec![("rocky1", 10), ("rocky3", 20)]), // rocky1:10 reused
        ];

        let result = check_duplicate_interface_link(&links, &bridges);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("rocky1"));
        assert!(err_msg.contains("interface index '10'"));
        assert!(err_msg.contains("already in use"));
    }

    #[test]
    fn test_bridge_device_not_defined() {
        // Bridge references a node that doesn't exist in manifest
        let devices = vec![
            Node {
                name: "rocky1".to_string(),
                model: NodeModel::RockyLinux,
                ..Default::default()
            },
            Node {
                name: "rocky2".to_string(),
                model: NodeModel::RockyLinux,
                ..Default::default()
            },
        ];

        let bridges = vec![create_bridge(
            "br1",
            vec![("rocky1", 5), ("rocky3", 10)], // rocky3 doesn't exist
        )];

        let result = check_bridge_device(&devices, &bridges);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("bridge"));
        assert!(err_msg.contains("rocky3"));
        assert!(err_msg.contains("not defined in devices"));
    }

    #[test]
    fn test_check_bridge_device_valid() -> Result<()> {
        // All bridge nodes are properly defined
        let devices = vec![
            Node {
                name: "rocky1".to_string(),
                model: NodeModel::RockyLinux,
                ..Default::default()
            },
            Node {
                name: "rocky2".to_string(),
                model: NodeModel::RockyLinux,
                ..Default::default()
            },
            Node {
                name: "rocky3".to_string(),
                model: NodeModel::RockyLinux,
                ..Default::default()
            },
        ];

        let bridges = vec![create_bridge(
            "br1",
            vec![("rocky1", 5), ("rocky2", 10), ("rocky3", 15)],
        )];

        check_bridge_device(&devices, &bridges)
    }
}
