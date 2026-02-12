use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};

use shared::data::NodeModel;
use topology::{LinkDetailed, Node};

/// Check if a device with a non-dedicated management interface
/// has the first interface defined in a connection
pub fn check_mgmt_usage(
    device_name: &str,
    first_interface_index: u8,
    links: &Vec<LinkDetailed>,
) -> Result<()> {
    for link in links {
        let (device, interface) = // no-fmt
        if device_name == link.node_a {
            (device_name, link.int_a_idx)
        } else if device_name == link.node_b {
            (device_name, link.int_b_idx)
        } else {
            continue; // this will skip to the next loop if device not matched in link
        };
        if device_name == device && first_interface_index == interface {
            bail!(
                "Manifest link - '{device}' interface '{interface}' overlaps with management interface",
            );
        }
    }
    Ok(())
}

/// Check for duplicate interface usage in device links
pub fn check_duplicate_interface_link(links: &Vec<LinkDetailed>) -> Result<()> {
    let mut device_int_map: HashMap<String, Vec<u8>> = HashMap::new();

    for link in links {
        check_device_interface(&link.node_a, link.int_a_idx, &mut device_int_map)?;
        check_device_interface(&link.node_b, link.int_b_idx, &mut device_int_map)?;
    }
    Ok(())
}
/// Helper function for `check_duplicate_interface_link` function
fn check_device_interface(
    device: &str,
    interface: u8,
    device_int_map: &mut HashMap<String, Vec<u8>>,
) -> Result<()> {
    match device_int_map.get_mut(device) {
        Some(interfaces) => {
            if interfaces.contains(&interface) {
                bail!("Manifest connection - '{device}' interface '{interface}' is already in use");
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

/// Check interface index bounds.
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
) -> Result<()> {
    // Calculate the valid interface range
    // Management is always at index 0
    // Reserved interfaces are at indices 1 to reserved_interface_count
    // Data interfaces start at (1 + reserved_interface_count)
    let first_data_interface_idx = 1 + reserved_interface_count;

    // Maximum interface index = first_data_interface_idx + data_interface_count - 1
    // For example: data_interface_count=52, reserved=0 -> first_data=1, max=52
    let max_interface_idx = first_data_interface_idx + data_interface_count - 1;

    for link in links {
        let (device, interface) = // no-fmt
        if device_name == link.node_a {
            (device_name, link.int_a_idx)
        } else if device_name == link.node_b {
            (device_name, link.int_b_idx)
        } else {
            continue; // this will skip to the next loop if device not matched in link
        };

        // Check if interface is in valid range
        if interface == 0 {
            if dedicated_management_interface {
                bail!(
                    "Manifest link - device '{device}' interface index 0 is the dedicated management interface and cannot be used for links"
                )
            }
            // If not dedicated, index 0 can be used for data, so it's valid
        } else if interface > 0 && interface < first_data_interface_idx {
            bail!(
                "Manifest link - device '{device}' interface index '{interface}' is a reserved interface (reserved_count: {reserved_interface_count}) and cannot be used for links"
            )
        } else if interface > max_interface_idx {
            bail!(
                "Manifest link - device '{device}' interface index '{interface}' exceeds the maximum interface index '{max_interface_idx}' for device model '{device_model}' (data_interface_count: {data_interface_count}, reserved: {reserved_interface_count})"
            )
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

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

        check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            52,   // data_interface_count
            0,    // reserved_interface_count
            true, // dedicated_management_interface
            &links,
        )
    }

    #[test]
    fn test_interface_bounds_exceeds_maximum() {
        // Rocky Linux with 52 data interfaces, trying to use interface 53
        let links = vec![create_link("rocky1", 53, "rocky2", 1)];

        let result = check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, true, &links);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("exceeds the maximum interface index"));
        assert!(err_msg.contains("52")); // max interface index
    }

    #[test]
    fn test_interface_bounds_management_interface_dedicated() {
        // Rocky Linux with dedicated management, trying to use interface 0
        let links = vec![create_link("rocky1", 0, "rocky2", 1)];

        let result = check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            52,
            0,
            true, // dedicated_management_interface = true
            &links,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("dedicated management interface"));
    }

    #[test]
    fn test_interface_bounds_management_interface_not_dedicated() -> Result<()> {
        // CentOS with non-dedicated management, using interface 0 is OK
        let links = vec![create_link("centos1", 0, "centos2", 0)];

        check_interface_bounds(
            "centos1",
            &NodeModel::CentosLinux,
            1,
            0,
            false, // dedicated_management_interface = false
            &links,
        )
    }

    #[test]
    fn test_interface_bounds_reserved_interface() {
        // Cisco IOS XRv9000 with 2 reserved interfaces
        let links = vec![create_link("xr1", 2, "xr2", 3)];

        let result = check_interface_bounds(
            "xr1",
            &NodeModel::CiscoIosxrv9000,
            31,
            2, // reserved_interface_count = 2 (indices 1 and 2)
            true,
            &links,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("reserved interface"));
    }

    #[test]
    fn test_interface_bounds_first_data_interface_after_reserved() -> Result<()> {
        // Cisco IOS XRv9000: interface 3 is first data interface
        let links = vec![create_link("xr1", 3, "xr2", 3)];

        check_interface_bounds(
            "xr1",
            &NodeModel::CiscoIosxrv9000,
            31,
            2, // reserved_interface_count = 2
            true,
            &links,
        )
    }

    #[test]
    fn test_interface_bounds_max_with_reserved() -> Result<()> {
        // Cisco IOS XRv9000: max data interface index = 3 + 31 - 1 = 33
        let links = vec![create_link("xr1", 33, "xr2", 3)];

        check_interface_bounds("xr1", &NodeModel::CiscoIosxrv9000, 31, 2, true, &links)
    }

    #[test]
    fn test_interface_bounds_link_not_involving_device() -> Result<()> {
        // Link doesn't involve the device being checked
        let links = vec![create_link("rocky1", 100, "rocky2", 100)];

        // Check device "rocky3" which is not in the link
        check_interface_bounds("rocky3", &NodeModel::RockyLinux, 52, 0, true, &links)
    }

    #[test]
    fn test_interface_bounds_empty_links() -> Result<()> {
        let links = vec![];

        check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, true, &links)
    }

    #[test]
    fn test_interface_bounds_minimum_data_interface() -> Result<()> {
        // First data interface (index 1) for Rocky Linux
        let links = vec![create_link("rocky1", 1, "rocky2", 1)];

        check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, true, &links)
    }

    #[test]
    fn test_interface_bounds_single_interface_device() -> Result<()> {
        // Device with only 1 data interface (data_interface_count=1)
        // Valid indices: 0 (mgmt), 1 (data)
        let links = vec![create_link("rocky1", 1, "rocky2", 1)];

        check_interface_bounds(
            "rocky1",
            &NodeModel::RockyLinux,
            1, // data_interface_count = 1
            0,
            true,
            &links,
        )
    }

    #[test]
    fn test_interface_bounds_single_interface_device_exceeds() {
        // Device with only 1 data interface, trying to use interface 2
        let links = vec![create_link("rocky1", 2, "rocky2", 1)];

        let result = check_interface_bounds("rocky1", &NodeModel::RockyLinux, 1, 0, true, &links);

        assert!(result.is_err());
    }
}
