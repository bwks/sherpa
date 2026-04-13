use tabled::{
    Table, Tabled,
    settings::{Alignment, Modify, Panel, Remove, Style, object::Rows, themes::BorderCorrection},
};

use crate::data::{
    BridgeInfo, DeviceInfo, ImageSummary, LabInfo, LinkInfo, NodeConfig, NodeInfo, ScannedImage,
};

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

    #[tabled(rename = "State")]
    state: String,

    #[tabled(rename = "Mgmt IPv4")]
    mgmt_ip: String,

    #[tabled(rename = "Mgmt IPv6")]
    mgmt_ipv6: String,

    #[tabled(rename = "VNC Port")]
    vnc_port: String,

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
pub fn render_devices_table(devices: &[DeviceInfo]) -> String {
    let rows: Vec<DeviceTableRow> = devices
        .iter()
        .map(|device| {
            let mgmt_ip = if device.mgmt_ipv4.is_empty() {
                "-".to_string()
            } else {
                device.mgmt_ipv4.clone()
            };
            // Format disks - join multiple disks with newline for table display
            let disks = if device.disks.is_empty() {
                "-".to_string()
            } else {
                device.disks.join("\n")
            };

            let vnc_port = match device.vnc_port {
                Some(port) => port.to_string(),
                None => "-".to_string(),
            };

            let mgmt_ipv6 = device.mgmt_ipv6.as_deref().unwrap_or("-").to_string();

            DeviceTableRow {
                device: device.name.clone(),
                model: device.model.to_string(),
                kind: device.kind.to_string(),
                state: device.state.to_string(),
                mgmt_ip,
                mgmt_ipv6,
                vnc_port,
                disks,
            }
        })
        .collect();

    Table::new(rows)
        .with(Style::modern())
        .with(Panel::header("Active Nodes"))
        .with(Modify::new(Rows::first()).with(Alignment::center()))
        .with(BorderCorrection::span())
        .to_string()
}

/// Represents a row in the links table
#[derive(Tabled)]
struct LinkTableRow {
    #[tabled(rename = "Node A")]
    node_a: String,

    #[tabled(rename = "Interface A")]
    int_a: String,

    #[tabled(rename = "Node B")]
    node_b: String,

    #[tabled(rename = "Interface B")]
    int_b: String,

    #[tabled(rename = "Type")]
    kind: String,
}

/// Renders a table of point-to-point links between nodes
pub fn render_links_table(links: &[LinkInfo]) -> String {
    let rows: Vec<LinkTableRow> = links
        .iter()
        .map(|link| LinkTableRow {
            node_a: link.node_a_name.clone(),
            int_a: link.int_a.clone(),
            node_b: link.node_b_name.clone(),
            int_b: link.int_b.clone(),
            kind: link.kind.clone(),
        })
        .collect();

    Table::new(rows)
        .with(Style::modern())
        .with(Panel::header("Links"))
        .with(Modify::new(Rows::first()).with(Alignment::center()))
        .with(BorderCorrection::span())
        .to_string()
}

/// Represents a row in the bridges table
#[derive(Tabled)]
struct BridgeTableRow {
    #[tabled(rename = "Bridge")]
    bridge_name: String,

    #[tabled(rename = "Network")]
    network_name: String,

    #[tabled(rename = "Connected Nodes")]
    connected_nodes: String,
}

/// Renders a table of shared bridges connecting multiple nodes
pub fn render_bridges_table(bridges: &[BridgeInfo]) -> String {
    let rows: Vec<BridgeTableRow> = bridges
        .iter()
        .map(|bridge| BridgeTableRow {
            bridge_name: bridge.bridge_name.clone(),
            network_name: bridge.network_name.clone(),
            connected_nodes: bridge.connected_nodes.join(", "),
        })
        .collect();

    Table::new(rows)
        .with(Style::modern())
        .with(Panel::header("Bridges"))
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
pub fn render_lab_info_table(lab_info: &LabInfo) -> String {
    let mut rows = vec![
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
            property: "Management Network".to_string(),
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
        LabInfoTableRow {
            property: "Loopback Network".to_string(),
            value: lab_info.loopback_network.to_string(),
        },
    ];

    if let Some(ref ipv6_net) = lab_info.ipv6_network {
        rows.push(LabInfoTableRow {
            property: "IPv6 Management Network".to_string(),
            value: ipv6_net.to_string(),
        });
    }
    if let Some(ref ipv6_gw) = lab_info.ipv6_gateway {
        rows.push(LabInfoTableRow {
            property: "IPv6 Gateway".to_string(),
            value: ipv6_gw.to_string(),
        });
    }
    if let Some(ref ipv6_rtr) = lab_info.ipv6_router {
        rows.push(LabInfoTableRow {
            property: "IPv6 Router".to_string(),
            value: ipv6_rtr.to_string(),
        });
    }

    Table::new(rows)
        .with(Style::modern())
        .with(Remove::row(Rows::first()))
        .with(Panel::header("Lab Info"))
        .with(Modify::new(Rows::first()).with(Alignment::center()))
        .with(BorderCorrection::span())
        .to_string()
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

/// Renders a table of images with their model, kind, version, and default status
pub fn render_images_table(images: &[ImageSummary]) -> String {
    Table::new(images)
        .with(Style::modern())
        .with(Panel::header("Images"))
        .with(Modify::new(Rows::first()).with(Alignment::center()))
        .with(BorderCorrection::span())
        .to_string()
}

/// Renders a table of scanned images with their model, version, kind, and status
pub fn render_scanned_images_table(images: &[ScannedImage]) -> String {
    Table::new(images)
        .with(Style::modern())
        .with(Panel::header("Scanned Images"))
        .with(Modify::new(Rows::first()).with(Alignment::center()))
        .with(BorderCorrection::span())
        .to_string()
}

/// Renders a two-column key-value table with all NodeConfig fields for an image
pub fn render_image_detail_table(image: &NodeConfig) -> String {
    let rows = vec![
        LabInfoTableRow {
            property: "Model".to_string(),
            value: image.model.to_string(),
        },
        LabInfoTableRow {
            property: "Kind".to_string(),
            value: image.kind.to_string(),
        },
        LabInfoTableRow {
            property: "Version".to_string(),
            value: image.version.clone(),
        },
        LabInfoTableRow {
            property: "Default".to_string(),
            value: image.default.to_string(),
        },
        LabInfoTableRow {
            property: "OS Variant".to_string(),
            value: image.os_variant.to_string(),
        },
        LabInfoTableRow {
            property: "BIOS".to_string(),
            value: image.bios.to_string(),
        },
        LabInfoTableRow {
            property: "CPU Architecture".to_string(),
            value: image.cpu_architecture.to_string(),
        },
        LabInfoTableRow {
            property: "CPU Model".to_string(),
            value: image.cpu_model.to_string(),
        },
        LabInfoTableRow {
            property: "CPU Count".to_string(),
            value: image.cpu_count.to_string(),
        },
        LabInfoTableRow {
            property: "Memory (MB)".to_string(),
            value: image.memory.to_string(),
        },
        LabInfoTableRow {
            property: "Machine Type".to_string(),
            value: image.machine_type.to_string(),
        },
        LabInfoTableRow {
            property: "VMX Enabled".to_string(),
            value: image.vmx_enabled.to_string(),
        },
        LabInfoTableRow {
            property: "HDD Bus".to_string(),
            value: image.hdd_bus.to_string(),
        },
        LabInfoTableRow {
            property: "CDROM".to_string(),
            value: image.cdrom.as_deref().unwrap_or("none").to_string(),
        },
        LabInfoTableRow {
            property: "CDROM Bus".to_string(),
            value: image.cdrom_bus.to_string(),
        },
        LabInfoTableRow {
            property: "Interface Type".to_string(),
            value: image.interface_type.to_string(),
        },
        LabInfoTableRow {
            property: "Interface Prefix".to_string(),
            value: image.interface_prefix.clone(),
        },
        LabInfoTableRow {
            property: "Interface MTU".to_string(),
            value: image.interface_mtu.to_string(),
        },
        LabInfoTableRow {
            property: "Data Interface Count".to_string(),
            value: image.data_interface_count.to_string(),
        },
        LabInfoTableRow {
            property: "First Interface Index".to_string(),
            value: image.first_interface_index.to_string(),
        },
        LabInfoTableRow {
            property: "Management Interface".to_string(),
            value: image.management_interface.to_string(),
        },
        LabInfoTableRow {
            property: "Dedicated Mgmt Interface".to_string(),
            value: image.dedicated_management_interface.to_string(),
        },
        LabInfoTableRow {
            property: "Reserved Interface Count".to_string(),
            value: image.reserved_interface_count.to_string(),
        },
        LabInfoTableRow {
            property: "Repo".to_string(),
            value: image.repo.as_deref().unwrap_or("none").to_string(),
        },
        LabInfoTableRow {
            property: "ZTP Enabled".to_string(),
            value: image.ztp_enable.to_string(),
        },
        LabInfoTableRow {
            property: "ZTP Method".to_string(),
            value: image.ztp_method.to_string(),
        },
        LabInfoTableRow {
            property: "ZTP Username".to_string(),
            value: image.ztp_username.as_deref().unwrap_or("none").to_string(),
        },
        LabInfoTableRow {
            property: "ZTP Password Auth".to_string(),
            value: image.ztp_password_auth.to_string(),
        },
    ];

    Table::new(rows)
        .with(Style::modern())
        .with(Remove::row(Rows::first()))
        .with(Panel::header("Image Detail"))
        .with(Modify::new(Rows::first()).with(Alignment::center()))
        .with(BorderCorrection::span())
        .to_string()
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

/// Represents a row in the server status table
#[derive(Tabled)]
struct ServerStatusRow {
    #[tabled(rename = "Server")]
    server: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "TLS")]
    tls: String,
}

/// Render a server status table
pub fn render_server_status_table(server: &str, status: &str, tls: &str) -> String {
    let row = ServerStatusRow {
        server: server.to_string(),
        status: status.to_string(),
        tls: tls.to_string(),
    };

    Table::new(vec![row])
        .with(Style::modern())
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
            state: NodeState::Running,
            mgmt_ipv4: "172.31.0.11".to_string(),
            mgmt_ipv6: None,
            vnc_port: None,
            disks: vec!["/var/lib/sherpa/labs/test/router01.qcow2".to_string()],
        }];

        let table = render_devices_table(&devices);

        // Check that table contains expected data
        assert!(table.contains("router01"));
        assert!(table.contains("172.31.0.11"));
        assert!(table.contains("cisco_cat8000v"));
        assert!(table.contains("virtual_machine"));
        assert!(table.contains("running"));
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
                state: NodeState::Running,
                mgmt_ipv4: "172.31.0.11".to_string(),
                mgmt_ipv6: None,
                vnc_port: Some(5900),
                disks: vec![
                    "/var/lib/sherpa/labs/test/router01.qcow2".to_string(),
                    "/var/lib/sherpa/labs/test/router01-disk2.qcow2".to_string(),
                ],
            },
            DeviceInfo {
                name: "switch01".to_string(),
                model: NodeModel::ArubaAoscx,
                kind: NodeKind::VirtualMachine,
                state: NodeState::Running,
                mgmt_ipv4: "172.31.0.12".to_string(),
                mgmt_ipv6: None,
                vnc_port: None,
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
            state: NodeState::Stopped,
            mgmt_ipv4: "".to_string(),
            mgmt_ipv6: None,
            vnc_port: None,
            disks: vec![],
        }];

        let table = render_devices_table(&devices);

        // Check that missing data is represented with "-"
        assert!(table.contains("container01"));
        assert!(table.contains("alpine_linux"));
        assert!(table.contains("stopped")); // state = Stopped
        assert!(table.contains("-")); // empty mgmt_ipv4
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
            loopback_network: "127.127.1.0/24".parse().unwrap(),
            ipv6_network: None,
            ipv6_gateway: None,
            ipv6_router: None,
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
        assert!(table.contains("Management Network"));
        assert!(table.contains("IPv4 Gateway"));
        assert!(table.contains("IPv4 Router"));

        // Check for modern style box-drawing characters
        assert!(table.contains("┌") || table.contains("│"));

        // Print the table for visual verification
        println!("\n{}", table);
    }

    #[test]
    fn test_render_image_detail_table() {
        let image = NodeConfig::get_model(NodeModel::CiscoCat8000v);

        let table = render_image_detail_table(&image);

        // Check that all property labels are present
        assert!(table.contains("Model"));
        assert!(table.contains("Kind"));
        assert!(table.contains("Version"));
        assert!(table.contains("Default"));
        assert!(table.contains("OS Variant"));
        assert!(table.contains("BIOS"));
        assert!(table.contains("CPU Architecture"));
        assert!(table.contains("CPU Model"));
        assert!(table.contains("CPU Count"));
        assert!(table.contains("Memory (MB)"));
        assert!(table.contains("Machine Type"));
        assert!(table.contains("VMX Enabled"));
        assert!(table.contains("HDD Bus"));
        assert!(table.contains("CDROM"));
        assert!(table.contains("CDROM Bus"));
        assert!(table.contains("Interface Type"));
        assert!(table.contains("Interface Prefix"));
        assert!(table.contains("Interface MTU"));
        assert!(table.contains("Data Interface Count"));
        assert!(table.contains("First Interface Index"));
        assert!(table.contains("Management Interface"));
        assert!(table.contains("Dedicated Mgmt Interface"));
        assert!(table.contains("Reserved Interface Count"));
        assert!(table.contains("Repo"));
        assert!(table.contains("ZTP Enabled"));
        assert!(table.contains("ZTP Method"));
        assert!(table.contains("ZTP Username"));
        assert!(table.contains("ZTP Password Auth"));

        // Check that model-specific values are present
        assert!(table.contains("cisco_cat8000v"));
        assert!(table.contains("virtual_machine"));

        // Check for header panel
        assert!(table.contains("Image Detail"));

        // Check for modern style box-drawing characters
        assert!(table.contains("┌") || table.contains("│"));

        println!("\n{}", table);
    }

    #[test]
    fn test_render_image_detail_table_container_model() {
        let image = NodeConfig::get_model(NodeModel::NokiaSrlinux);

        let table = render_image_detail_table(&image);

        // Check container-specific values
        assert!(table.contains("nokia_srlinux"));
        assert!(table.contains("container"));

        // Repo should be populated for container models
        assert!(!table.contains("Repo") || !table.contains("none") || table.contains("ghcr.io"));

        println!("\n{}", table);
    }

    #[test]
    fn test_render_image_detail_table_optional_fields() {
        let image = NodeConfig::default();

        let table = render_image_detail_table(&image);

        // Optional fields should show "none" when not set
        assert!(table.contains("none")); // repo, cdrom, ztp_username are None by default

        println!("\n{}", table);
    }
}
