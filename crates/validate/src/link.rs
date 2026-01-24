use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};

use data::NodeModel;
use topology::{LinkDetailed, LinkExpanded, Node};

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
pub fn check_link_device(devices: &[Node], links: &Vec<LinkExpanded>) -> Result<()> {
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
/// - less than first_interface_index
/// - greater than interface_count
pub fn check_interface_bounds(
    device_name: &str,
    device_model: &NodeModel,
    first_interface_index: u8,
    interface_count: u8,
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

        if interface < first_interface_index {
            bail!(
                "Manifest link - device '{device}' has interface index '{interface}' defined, which is lower than the '{device_model}' first interface index '{first_interface_index}'"
            )
        } else if interface > interface_count {
            bail!(
                "Manifest link - device '{device}' has interface index '{interface}' defined, which is higher than the '{device_model}' configured number of interfaces '{interface_count}'"
            )
        }
    }

    Ok(())
}
