use tabled::{
    Table, Tabled,
    settings::{Alignment, Modify, Panel, Remove, Style, object::Rows, themes::BorderCorrection},
};

use crate::data::{DeviceInfo, LabInfo, NodeInfo};

/// Represents a row in the nodes table
#[derive(Tabled)]
struct NodeTableRow {
    #[tabled(rename = "Node")]
    node: String,

    #[tabled(rename = "Mgmt IP")]
    mgmt_ip: String,

    #[tabled(rename = "Connection")]
    connection: String,

    #[tabled(rename = "Node Model")]
    node_model: String,
}

/// Renders a table of nodes with their management IP, connection info, and model
///
/// # Arguments
/// * `nodes` - Slice of NodeInfo structs to display in the table
///
/// # Returns
/// A formatted table string using modern Unicode box-drawing characters
///
/// # Example
/// ```
/// use shared::data::up::{NodeInfo, NodeStatus};
/// use shared::data::node::NodeModel;
/// use shared::util::table::render_nodes_table;
///
/// let nodes = vec![
///     NodeInfo {
///         name: "dev01".to_string(),
///         kind: "VirtualMachine".to_string(),
///         model: NodeModel::ArubaAoscx,
///         status: NodeState::Running,
///         ip_address: Some("172.31.0.11".to_string()),
///         ssh_port: Some(22),
///     },
/// ];
///
/// let table = render_nodes_table(&nodes);
/// assert!(table.contains("dev01"));
/// assert!(table.contains("172.31.0.11"));
/// ```
pub fn render_nodes_table(nodes: &[NodeInfo]) -> String {
    let rows: Vec<NodeTableRow> = nodes
        .iter()
        .map(|node| {
            let mgmt_ip = node.ip_address.as_deref().unwrap_or("-");
            let connection = match (&node.ip_address, node.ssh_port) {
                (Some(ip), Some(port)) => format!("{}:{}", ip, port),
                _ => "-".to_string(),
            };
            let node_model = node.model.to_string();

            NodeTableRow {
                node: node.name.clone(),
                mgmt_ip: mgmt_ip.to_string(),
                connection,
                node_model,
            }
        })
        .collect();

    Table::new(rows)
        .with(Style::modern())
        .with(Panel::header("Nodes"))
        .with(Modify::new(Rows::first()).with(Alignment::center()))
        .with(BorderCorrection::span())
        .to_string()
}

/// Represents a row in the devices table
#[derive(Tabled)]
struct DeviceTableRow {
    #[tabled(rename = "Device")]
    device: String,

    #[tabled(rename = "Model")]
    model: String,

    #[tabled(rename = "Kind")]
    kind: String,

    #[tabled(rename = "Active")]
    active: String,

    #[tabled(rename = "Mgmt IP")]
    mgmt_ip: String,

    #[tabled(rename = "Disks")]
    disks: String,
}

/// Renders a table of devices with their model, kind, active status, and management IP
///
/// # Arguments
/// * `devices` - Slice of DeviceInfo structs to display in the table
///
/// # Returns
/// A formatted table string using modern Unicode box-drawing characters
///
/// # Example
/// ```
/// use shared::data::{DeviceInfo, NodeKind, NodeModel};
/// use shared::util::table::render_devices_table;
///
/// let devices = vec![
///     DeviceInfo {
///         name: "router01".to_string(),
///         model: NodeModel::CiscoCat8000v,
///         kind: NodeKind::VirtualMachine,
///         active: true,
///         mgmt_ip: "172.31.0.11".to_string(),
///         disks: vec![],
///     },
/// ];
///
/// let table = render_devices_table(&devices);
/// assert!(table.contains("router01"));
/// assert!(table.contains("172.31.0.11"));
/// ```
pub fn render_devices_table(devices: &[DeviceInfo]) -> String {
    let rows: Vec<DeviceTableRow> = devices
        .iter()
        .map(|device| {
            let mgmt_ip = if device.mgmt_ip.is_empty() {
                "-".to_string()
            } else {
                device.mgmt_ip.clone()
            };
            let active = if device.active { "Yes" } else { "No" };

            // Format disks - join multiple disks with newline for table display
            let disks = if device.disks.is_empty() {
                "-".to_string()
            } else {
                device.disks.join("\n")
            };

            DeviceTableRow {
                device: device.name.clone(),
                model: device.model.to_string(),
                kind: device.kind.to_string(),
                active: active.to_string(),
                mgmt_ip,
                disks,
            }
        })
        .collect();

    Table::new(rows)
        .with(Style::modern())
        .with(Panel::header("Active Devices"))
        .with(Modify::new(Rows::first()).with(Alignment::center()))
        .with(BorderCorrection::span())
        .to_string()
}

/// Represents a row in the lab info table (two-column format)
#[derive(Tabled)]
struct LabInfoTableRow {
    #[tabled(rename = "Property")]
    property: String,

    #[tabled(rename = "Value")]
    value: String,
}

/// Renders a two-column table of lab information
///
/// # Arguments
/// * `lab_info` - Reference to LabInfo struct to display
///
/// # Returns
/// A formatted table string using modern Unicode box-drawing characters
///
/// # Example
/// ```
/// use shared::data::LabInfo;
/// use shared::util::table::render_lab_info_table;
/// use std::net::Ipv4Addr;
/// use ipnet::Ipv4Net;
///
/// let lab_info = LabInfo {
///     id: "039ab286".to_string(),
///     name: "simple-ceos-test".to_string(),
///     user: "admin".to_string(),
///     ipv4_network: "172.31.0.0/24".parse().unwrap(),
///     ipv4_gateway: "172.31.0.1".parse().unwrap(),
///     ipv4_router: "172.31.0.2".parse().unwrap(),
/// };
///
/// let table = render_lab_info_table(&lab_info);
/// assert!(table.contains("039ab286"));
/// assert!(table.contains("simple-ceos-test"));
/// ```
pub fn render_lab_info_table(lab_info: &LabInfo) -> String {
    let rows = vec![
        LabInfoTableRow {
            property: "ID".to_string(),
            value: lab_info.id.clone(),
        },
        LabInfoTableRow {
            property: "Name".to_string(),
            value: lab_info.name.clone(),
        },
        LabInfoTableRow {
            property: "User".to_string(),
            value: lab_info.user.clone(),
        },
        LabInfoTableRow {
            property: "IPv4 Network".to_string(),
            value: lab_info.ipv4_network.to_string(),
        },
        LabInfoTableRow {
            property: "IPv4 Gateway".to_string(),
            value: lab_info.ipv4_gateway.to_string(),
        },
        LabInfoTableRow {
            property: "IPv4 Router".to_string(),
            value: lab_info.ipv4_router.to_string(),
        },
    ];

    Table::new(rows)
        .with(Style::modern())
        .with(Remove::row(Rows::first()))
        .with(Panel::header("Lab Info"))
        .with(Modify::new(Rows::first()).with(Alignment::center()))
        .with(BorderCorrection::span())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{NodeKind, NodeModel, NodeState};

    #[test]
    fn test_render_single_node() {
        let nodes = vec![NodeInfo {
            name: "dev01".to_string(),
            kind: "VirtualMachine".to_string(),
            model: NodeModel::ArubaAoscx,
            status: NodeState::Running,
            ip_address: Some("172.31.0.11".to_string()),
            ssh_port: Some(22),
        }];

        let table = render_nodes_table(&nodes);

        // Check that table contains expected data
        assert!(table.contains("dev01"));
        assert!(table.contains("172.31.0.11"));
        assert!(table.contains("172.31.0.11:22"));
        assert!(table.contains("aruba_aoscx"));

        // Check for modern style box-drawing characters
        assert!(table.contains("┌") || table.contains("│"));
    }

    #[test]
    fn test_render_multiple_nodes() {
        let nodes = vec![
            NodeInfo {
                name: "dev01".to_string(),
                kind: "VirtualMachine".to_string(),
                model: NodeModel::ArubaAoscx,
                status: NodeState::Running,
                ip_address: Some("172.31.0.11".to_string()),
                ssh_port: Some(22),
            },
            NodeInfo {
                name: "router01".to_string(),
                kind: "VirtualMachine".to_string(),
                model: NodeModel::CiscoCat8000v,
                status: NodeState::Running,
                ip_address: Some("172.31.0.12".to_string()),
                ssh_port: Some(22),
            },
        ];

        let table = render_nodes_table(&nodes);
        println!("\n{}", table);

        // Check that all nodes are present
        assert!(table.contains("dev01"));
        assert!(table.contains("router01"));
        assert!(table.contains("172.31.0.11"));
        assert!(table.contains("172.31.0.12"));
        assert!(table.contains("aruba_aoscx"));
        assert!(table.contains("cisco_cat8000v"));
    }

    #[test]
    fn test_render_node_missing_ip() {
        let nodes = vec![NodeInfo {
            name: "dev01".to_string(),
            kind: "Container".to_string(),
            model: NodeModel::AlpineLinux,
            status: NodeState::Starting,
            ip_address: None,
            ssh_port: None,
        }];

        let table = render_nodes_table(&nodes);

        // Check that missing data is represented with "-"
        assert!(table.contains("dev01"));
        assert!(table.contains("alpine_linux"));
        // The table should have "-" for missing IP and connection
        assert!(table.matches("-").count() >= 2);
    }

    #[test]
    fn test_render_empty_nodes() {
        let nodes: Vec<NodeInfo> = vec![];
        let table = render_nodes_table(&nodes);

        // Empty table should still have headers
        assert!(table.contains("Node"));
        assert!(table.contains("Mgmt IP"));
        assert!(table.contains("Connection"));
        assert!(table.contains("Node Model"));
    }

    #[test]
    fn test_render_single_device() {
        let devices = vec![DeviceInfo {
            name: "router01".to_string(),
            model: NodeModel::CiscoCat8000v,
            kind: NodeKind::VirtualMachine,
            active: true,
            mgmt_ip: "172.31.0.11".to_string(),
            disks: vec!["/var/lib/sherpa/labs/test/router01.qcow2".to_string()],
        }];

        let table = render_devices_table(&devices);

        // Check that table contains expected data
        assert!(table.contains("router01"));
        assert!(table.contains("172.31.0.11"));
        assert!(table.contains("cisco_cat8000v"));
        assert!(table.contains("virtual_machine"));
        assert!(table.contains("Yes"));
        assert!(table.contains("router01.qcow2"));

        // Check for modern style box-drawing characters
        assert!(table.contains("┌") || table.contains("│"));
    }

    #[test]
    fn test_render_multiple_devices() {
        let devices = vec![
            DeviceInfo {
                name: "router01".to_string(),
                model: NodeModel::CiscoCat8000v,
                kind: NodeKind::VirtualMachine,
                active: true,
                mgmt_ip: "172.31.0.11".to_string(),
                disks: vec![
                    "/var/lib/sherpa/labs/test/router01.qcow2".to_string(),
                    "/var/lib/sherpa/labs/test/router01-disk2.qcow2".to_string(),
                ],
            },
            DeviceInfo {
                name: "switch01".to_string(),
                model: NodeModel::ArubaAoscx,
                kind: NodeKind::VirtualMachine,
                active: true,
                mgmt_ip: "172.31.0.12".to_string(),
                disks: vec!["/var/lib/sherpa/labs/test/switch01.qcow2".to_string()],
            },
        ];

        let table = render_devices_table(&devices);
        println!("\n{}", table);

        // Check that all devices are present
        assert!(table.contains("router01"));
        assert!(table.contains("switch01"));
        assert!(table.contains("172.31.0.11"));
        assert!(table.contains("172.31.0.12"));
        assert!(table.contains("cisco_cat8000v"));
        assert!(table.contains("aruba_aoscx"));
        assert!(table.contains("router01.qcow2"));
        assert!(table.contains("switch01.qcow2"));
    }

    #[test]
    fn test_render_device_missing_ip() {
        let devices = vec![DeviceInfo {
            name: "container01".to_string(),
            model: NodeModel::AlpineLinux,
            kind: NodeKind::Container,
            active: false,
            mgmt_ip: "".to_string(),
            disks: vec![],
        }];

        let table = render_devices_table(&devices);

        // Check that missing data is represented with "-"
        assert!(table.contains("container01"));
        assert!(table.contains("alpine_linux"));
        assert!(table.contains("No")); // active = false
        assert!(table.contains("-")); // empty mgmt_ip
    }

    #[test]
    fn test_render_empty_devices() {
        let devices: Vec<DeviceInfo> = vec![];
        let table = render_devices_table(&devices);

        // Empty table should still have headers
        assert!(table.contains("Device"));
        assert!(table.contains("Model"));
        assert!(table.contains("Kind"));
        assert!(table.contains("Active"));
        assert!(table.contains("Mgmt IP"));
        assert!(table.contains("Disks"));
    }

    #[test]
    fn test_render_lab_info_table() {
        use std::net::Ipv4Addr;

        let lab_info = LabInfo {
            id: "039ab286".to_string(),
            name: "simple-ceos-test".to_string(),
            user: "admin".to_string(),
            ipv4_network: "172.31.0.0/24".parse().unwrap(),
            ipv4_gateway: Ipv4Addr::new(172, 31, 0, 1),
            ipv4_router: Ipv4Addr::new(172, 31, 0, 2),
        };

        let table = render_lab_info_table(&lab_info);

        // Check that all values are present
        assert!(table.contains("039ab286"));
        assert!(table.contains("simple-ceos-test"));
        assert!(table.contains("admin"));
        assert!(table.contains("172.31.0.0/24"));
        assert!(table.contains("172.31.0.1"));
        assert!(table.contains("172.31.0.2"));

        // Check that properties are present
        assert!(table.contains("ID"));
        assert!(table.contains("Name"));
        assert!(table.contains("User"));
        assert!(table.contains("IPv4 Network"));
        assert!(table.contains("IPv4 Gateway"));
        assert!(table.contains("IPv4 Router"));

        // Check for modern style box-drawing characters
        assert!(table.contains("┌") || table.contains("│"));

        // Print the table for visual verification
        println!("\n{}", table);
    }
}

/// Represents a row in the certificates table
#[derive(Tabled)]
struct CertTableRow {
    #[tabled(rename = "Server")]
    server: String,

    #[tabled(rename = "Subject")]
    subject: String,

    #[tabled(rename = "Valid Until")]
    valid_until: String,
}

/// Certificate information for table display
pub struct CertificateTableInfo {
    pub server: String,
    pub subject: String,
    pub valid_until: String,
}

/// Renders a table of trusted certificates
///
/// # Arguments
/// * `certs` - Slice of CertificateTableInfo structs to display in the table
///
/// # Returns
/// A formatted table string using modern Unicode box-drawing characters
///
/// # Example
/// ```
/// use shared::util::table::{render_certificates_table, CertificateTableInfo};
///
/// let certs = vec![
///     CertificateTableInfo {
///         server: "10.100.58.10:3030".to_string(),
///         subject: "Sherpa Server".to_string(),
///         valid_until: "Feb 16 23:05:43 2027".to_string(),
///     },
/// ];
///
/// let table = render_certificates_table(&certs);
/// assert!(table.contains("10.100.58.10:3030"));
/// assert!(table.contains("Sherpa Server"));
/// ```
pub fn render_certificates_table(certs: &[CertificateTableInfo]) -> String {
    let rows: Vec<CertTableRow> = certs
        .iter()
        .map(|cert| CertTableRow {
            server: cert.server.clone(),
            subject: cert.subject.clone(),
            valid_until: cert.valid_until.clone(),
        })
        .collect();

    Table::new(rows)
        .with(Style::modern())
        .with(Panel::header("Trusted Server Certificates"))
        .with(Modify::new(Rows::first()).with(Alignment::center()))
        .with(BorderCorrection::span())
        .to_string()
}
