use std::collections::HashMap;

use crate::topology::Connection;

// Check if a device with a non-dedicated management interface
// has the first interface defined in a connection
pub fn check_mgmt_usage(
    device_name: &str,
    first_interface_index: u8,
    connections: Vec<Connection>,
) -> bool {
    for connection in connections {
        // Device A
        if device_name == connection.device_a {
            if first_interface_index == connection.interface_a {
                println!(
                    "Device: {} interface: {} overlaps with management interface",
                    connection.device_a, connection.interface_a
                );
                return true;
            }
        // Device B
        } else {
            if first_interface_index == connection.interface_b {
                println!(
                    "Device: {} interface: {} overlaps with management interface",
                    connection.device_b, connection.interface_b
                );
                return true;
            }
        }
    }
    false
}

// Check for duplicate interface usage in device connections
pub fn check_duplicate_interface_connecion(connections: Vec<Connection>) -> bool {
    let mut device_int_map: HashMap<String, Vec<u8>> = HashMap::new();
    for connection in connections {
        // Device A
        if device_int_map.contains_key(&connection.device_a) {
            if let Some(value) = device_int_map.get_mut(&connection.device_a) {
                if value.contains(&connection.interface_a) {
                    println!(
                        "Device: {} interface: {} used more than once",
                        connection.device_a, connection.interface_a
                    );
                    return true;
                } else {
                    value.push(connection.interface_a)
                }
            }
        } else {
            device_int_map.insert(connection.device_a, vec![connection.interface_a]);
        }
        // Device B
        if device_int_map.contains_key(&connection.device_b) {
            if let Some(value) = device_int_map.get_mut(&connection.device_b) {
                if value.contains(&connection.interface_b) {
                    println!(
                        "Device: {} interface: {} used more than once",
                        connection.device_b, connection.interface_b
                    );
                    return true;
                } else {
                    value.push(connection.interface_b)
                }
            }
        } else {
            device_int_map.insert(connection.device_b, vec![connection.interface_b]);
        }
    }
    false
}

// Check interface bounds
pub fn check_interface_bounds() -> bool {
    // Check if interface is < lowest interface index

    // Check if interface is > interface count
    false
}
