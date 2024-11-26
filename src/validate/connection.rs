use std::collections::{HashMap, HashSet};

use anyhow::{bail, Result};

use crate::data::DeviceModels;
use crate::topology::{Connection, Device};

/// Check if a device with a non-dedicated management interface
/// has the first interface defined in a connection
pub fn check_mgmt_usage(
    device_name: &str,
    first_interface_index: u8,
    connections: &Vec<Connection>,
) -> Result<()> {
    for connection in connections {
        let (device, interface) = // no-fmt 
        if device_name == connection.device_a {
            (device_name, connection.interface_a)
        } else if device_name == connection.device_b {
            (device_name, connection.interface_b)
        } else {
            continue; // this will skip to the next loop if device not matched in connection
        };
        if device_name == device && first_interface_index == interface {
            bail!(
                "Manifest connection - '{device}' interface '{interface}' overlaps with management interface",
            );
        }
    }
    Ok(())
}

/// Check for duplicate interface usage in device connections
pub fn check_duplicate_interface_connection(connections: &Vec<Connection>) -> Result<()> {
    let mut device_int_map: HashMap<String, Vec<u8>> = HashMap::new();

    for connection in connections {
        check_device_interface(
            &connection.device_a,
            connection.interface_a,
            &mut device_int_map,
        )?;
        check_device_interface(
            &connection.device_b,
            connection.interface_b,
            &mut device_int_map,
        )?;
    }
    Ok(())
}
/// Helper function for `check_duplicate_interface_connection` function
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

/// Check devices defined in connections are defined as top level devices
pub fn check_connection_device(devices: &[Device], connections: &Vec<Connection>) -> Result<()> {
    let unique_devices: Vec<String> = devices.iter().map(|d| d.name.clone()).collect();
    let mut unique_device_connection: HashSet<String> = HashSet::new();
    for connection in connections {
        unique_device_connection.insert(connection.device_a.clone());
        unique_device_connection.insert(connection.device_b.clone());
    }
    for device in &unique_device_connection {
        if !unique_devices.contains(device) {
            bail!(
                "Manifest connection - '{device}' defined in connections, not defined in devices"
            );
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
    device_model: &DeviceModels,
    first_interface_index: u8,
    interface_count: u8,
    connections: &Vec<Connection>,
) -> Result<()> {
    for connection in connections {
        let (device, interface) = // no-fmt 
        if device_name == connection.device_a {
            (device_name, connection.interface_a)
        } else if device_name == connection.device_b {
            (device_name, connection.interface_b)
        } else {
            continue; // this will skip to the next loop if device not matched in connection
        };

        if interface < first_interface_index {
            bail!("Manifest connection - device '{device}' has interface index '{interface}' defined, which is lower than the '{device_model}' first interface index '{first_interface_index}'")
        } else if interface > interface_count {
            bail!("Manifest connection - device '{device}' has interface index '{interface}' defined, which is higher than the '{device_model}' configured number of interfaces '{interface_count}'")
        }
    }

    Ok(())
}
