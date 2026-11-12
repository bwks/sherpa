//! Lab tables: a lab's info, nodes, devices, links, bridges, provisioning results and
//! Tailscale gateway.

use shared::data::{
    BridgeInfo, DeviceInfo, LabInfo, LinkInfo, NodeInfo, ProvisionResult, TailnetStatus,
};
use shared::util::{TableLayout, render_table};
use tracing::instrument;

const NODE_HEADERS: [&str; 4] = ["Node", "Mgmt IP", "Connection", "Node Model"];
const DEVICE_HEADERS: [&str; 10] = [
    "Device",
    "Model",
    "Kind",
    "State",
    "Mgmt IPv4",
    "Mgmt IPv6",
    "CPU",
    "Memory (MB)",
    "VNC Port",
    "Disks",
];
const LINK_HEADERS: [&str; 5] = ["Node A", "Interface A", "Node B", "Interface B", "Type"];
const BRIDGE_HEADERS: [&str; 3] = ["Bridge", "Network", "Connected Nodes"];
const PROVISION_HEADERS: [&str; 3] = ["Node", "Status", "Detail"];
const TAILNET_HEADERS: [&str; 2] = ["Tailscale", "Details"];
/// The lab info table has no header row: its first column names each row.
const LAB_INFO_HEADERS: [&str; 2] = ["Property", "Value"];
/// The widest the Tailscale table may be, borders and padding included.
const TAILNET_WIDTH: usize = 80;

/// Renders a table of nodes with their management IP, connection info, and model. Each node is
/// named `<name>.<lab_id>`.
#[must_use]
pub fn render_nodes_table(nodes: &[NodeInfo], lab_id: &str) -> String {
    let rows: Vec<[String; 4]> = nodes.iter().map(|node| node_cells(node, lab_id)).collect();
    render_table(
        NODE_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Nodes"),
            ..TableLayout::default()
        },
    )
}

fn node_cells(node: &NodeInfo, lab_id: &str) -> [String; 4] {
    let connection = match (&node.ip_address, node.ssh_port) {
        (Some(ip), Some(port)) => format!("{ip}:{port}"),
        _ => "-".to_string(),
    };
    [
        format!("{}.{lab_id}", node.name),
        node.ip_address.as_deref().unwrap_or("-").to_string(),
        connection,
        node.model.to_string(),
    ]
}

/// Renders a table of devices with their model, kind, state, management IPs, vCPUs, memory,
/// VNC port and disks. Each device is named `<name>.<lab_id>`.
#[must_use]
pub fn render_devices_table(devices: &[DeviceInfo], lab_id: &str) -> String {
    let rows: Vec<[String; 10]> = devices
        .iter()
        .map(|device| device_cells(device, lab_id))
        .collect();
    render_table(
        DEVICE_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Active Nodes"),
            ..TableLayout::default()
        },
    )
}

fn device_cells(device: &DeviceInfo, lab_id: &str) -> [String; 10] {
    let mgmt_ip = if device.mgmt_ipv4.is_empty() {
        "-".to_string()
    } else {
        device.mgmt_ipv4.clone()
    };
    // Multiple disks are joined with a newline, one per line in the cell.
    let disks = if device.disks.is_empty() {
        "-".to_string()
    } else {
        device.disks.join("\n")
    };
    let vnc_port = match device.vnc_port {
        Some(port) => port.to_string(),
        None => "-".to_string(),
    };
    // Containers have no limits, so they show "-".
    let cpu_count = match device.cpu_count {
        Some(count) => count.to_string(),
        None => "-".to_string(),
    };
    let memory = match device.memory {
        Some(memory) => memory.to_string(),
        None => "-".to_string(),
    };
    [
        format!("{}.{lab_id}", device.name),
        device.model.to_string(),
        device.kind.to_string(),
        device.state.to_string(),
        mgmt_ip,
        device.mgmt_ipv6.as_deref().unwrap_or("-").to_string(),
        cpu_count,
        memory,
        vnc_port,
        disks,
    ]
}

/// Renders a table of point-to-point links between nodes
#[must_use]
pub fn render_links_table(links: &[LinkInfo]) -> String {
    let rows: Vec<[String; 5]> = links.iter().map(link_cells).collect();
    render_table(
        LINK_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Links"),
            ..TableLayout::default()
        },
    )
}

fn link_cells(link: &LinkInfo) -> [String; 5] {
    [
        link.node_a_name.clone(),
        link.int_a.clone(),
        link.node_b_name.clone(),
        link.int_b.clone(),
        link.kind.to_string(),
    ]
}

/// Renders a table of shared bridges connecting multiple nodes
#[must_use]
pub fn render_bridges_table(bridges: &[BridgeInfo]) -> String {
    let rows: Vec<[String; 3]> = bridges.iter().map(bridge_cells).collect();
    render_table(
        BRIDGE_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Bridges"),
            ..TableLayout::default()
        },
    )
}

fn bridge_cells(bridge: &BridgeInfo) -> [String; 3] {
    [
        bridge.bridge_name.clone(),
        bridge.network_name.clone(),
        bridge.connected_nodes.join(", "),
    ]
}

/// Renders the result of running provisioners on a lab's nodes
#[must_use]
pub fn render_provision_table(results: &[ProvisionResult], lab_id: &str) -> String {
    let rows: Vec<[String; 3]> = results
        .iter()
        .map(|result| provision_cells(result, lab_id))
        .collect();
    render_table(
        PROVISION_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Provisioning"),
            ..TableLayout::default()
        },
    )
}

fn provision_cells(result: &ProvisionResult, lab_id: &str) -> [String; 3] {
    [
        format!("{}.{lab_id}", result.node),
        result.status.to_string(),
        result.detail.clone(),
    ]
}

/// Renders a two-column table of lab information. The optional properties are left out when
/// they are not set.
#[must_use]
pub fn render_lab_info_table(lab_info: &LabInfo) -> String {
    let mut rows = vec![
        ["ID".to_owned(), lab_info.id.clone()],
        ["Name".to_owned(), lab_info.name.clone()],
        ["User".to_owned(), lab_info.user.clone()],
        [
            "Management Network".to_owned(),
            lab_info.ipv4_network.to_string(),
        ],
        ["IPv4 Gateway".to_owned(), lab_info.ipv4_gateway.to_string()],
        ["IPv4 Router".to_owned(), lab_info.ipv4_router.to_string()],
        [
            "Loopback Network".to_owned(),
            lab_info.loopback_network.to_string(),
        ],
    ];
    if let Some(description) = &lab_info.description {
        rows.insert(2, ["Description".to_owned(), description.clone()]);
    }
    let optional = [
        (
            "IPv6 Management Network",
            lab_info.ipv6_network.map(|net| net.to_string()),
        ),
        (
            "IPv6 Gateway",
            lab_info.ipv6_gateway.map(|addr| addr.to_string()),
        ),
        (
            "IPv6 Router",
            lab_info.ipv6_router.map(|addr| addr.to_string()),
        ),
        ("Client Host", lab_info.client_host.clone()),
        ("Client Folder", lab_info.client_folder.clone()),
    ];
    rows.extend(
        optional
            .into_iter()
            .filter_map(|(property, value)| Some([property.to_owned(), value?])),
    );
    render_table(
        LAB_INFO_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Lab Info"),
            header_row: false,
            ..TableLayout::default()
        },
    )
}

/// Shows connection and approval separately; connected does not imply reachable. The table is
/// bounded to 80 characters, and the details column wraps first so the labels stay readable.
#[instrument(skip(status), level = "debug")]
pub fn render_tailnet_table(status: &TailnetStatus) -> String {
    let rows = [
        ["Gateway".to_string(), format!("{:?}", status.state)],
        [
            "Route approval".to_string(),
            format!("{:?}", status.approval),
        ],
        ["Management routes".to_string(), status.routes.join(", ")],
        [
            "Tailscale addresses".to_string(),
            status.tailscale_ips.join(", "),
        ],
        ["Warnings".to_string(), status.warnings.join("\n")],
    ];
    render_table(
        TAILNET_HEADERS,
        &rows,
        &TableLayout {
            wrap_width: Some(TAILNET_WIDTH),
            ..TableLayout::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;
    use shared::data::{NodeKind, NodeModel, NodeState, ProvisionStatus};

    #[test]
    fn provision_table_shows_each_node_with_its_status() {
        let results = vec![
            ProvisionResult {
                node: "dev01".to_string(),
                status: ProvisionStatus::Provisioned,
                detail: String::new(),
            },
            ProvisionResult {
                node: "dev02".to_string(),
                status: ProvisionStatus::Failed,
                detail: "exit status 3 on dev02.abcd1234".to_string(),
            },
        ];
        let table = render_provision_table(&results, "abcd1234");
        assert!(table.contains("Provisioning"), "{table}");
        assert!(table.contains("dev01.abcd1234"), "{table}");
        assert!(table.contains("provisioned"), "{table}");
        assert!(table.contains("failed"), "{table}");
        assert!(table.contains("exit status 3"), "{table}");
    }

    #[test]
    fn tailnet_table_wraps_pending_route_warning() {
        let status = TailnetStatus::from_daemon(
            r#"{"BackendState":"Running","Self":{"AllowedIPs":[]}}"#,
            vec!["172.31.3.0/24".into(), "fd00:b00b:0:3::/64".into()],
        )
        .unwrap();

        let table = render_tailnet_table(&status);
        assert!(table.lines().all(|line| line.chars().count() <= 80));
        assert!(table.contains("Management routes"));
        assert!(table.contains("Tailscale addresses"));
        assert!(table.contains("Pending"));

        // Recover cell content to check that wrapping loses no warning text,
        // including long URLs that must be split across lines.
        let details: String = table
            .lines()
            .filter_map(|line| line.split('│').nth(2))
            .flat_map(|cell| cell.chars().filter(|c| !c.is_whitespace()))
            .collect();
        let warning: String = status.warnings[0]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(details.contains(&warning));
    }

    #[test]
    fn tailnet_table_preserves_multiple_warnings() {
        let mut status = TailnetStatus::unavailable(vec!["172.31.3.0/24".into()]);
        status.warnings.push("Another gateway warning.".into());

        let table = render_tailnet_table(&status);
        assert!(table.lines().all(|line| line.chars().count() <= 80));
        assert!(table.contains("Unavailable"));
        assert!(table.contains("Another gateway warning."));
        let details: String = table
            .lines()
            .filter_map(|line| line.split('│').nth(2))
            .flat_map(|cell| cell.chars().filter(|c| !c.is_whitespace()))
            .collect();
        for warning in status.warnings {
            let warning: String = warning.chars().filter(|c| !c.is_whitespace()).collect();
            assert!(details.contains(&warning));
        }
    }

    #[test]
    fn tailnet_table_renders_approved_status_without_warnings() {
        let status = TailnetStatus::from_daemon(
            r#"{"BackendState":"Running","TailscaleIPs":["100.64.0.1"],"Self":{"AllowedIPs":["172.31.3.0/24"]}}"#,
            vec!["172.31.3.0/24".into()],
        )
        .unwrap();

        let table = render_tailnet_table(&status);
        assert!(table.lines().all(|line| line.chars().count() <= 80));
        assert!(table.contains("Connected"));
        assert!(table.contains("Approved"));
        assert!(table.contains("172.31.3.0/24"));
        assert!(table.contains("100.64.0.1"));
        assert!(table.contains("Warnings"));
        assert_eq!(table.lines().count(), 13);
    }

    #[test]
    fn test_render_single_node() {
        let nodes = vec![NodeInfo {
            name: "dev01".to_string(),
            kind: NodeKind::VirtualMachine,
            model: NodeModel::ArubaAoscx,
            status: NodeState::Running,
            ip_address: Some("172.31.0.11".to_string()),
            ssh_port: Some(22),
        }];

        let table = render_nodes_table(&nodes, "a1b2c3d4");

        // Check that table contains expected data
        assert!(table.contains("dev01.a1b2c3d4"));
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
                kind: NodeKind::VirtualMachine,
                model: NodeModel::ArubaAoscx,
                status: NodeState::Running,
                ip_address: Some("172.31.0.11".to_string()),
                ssh_port: Some(22),
            },
            NodeInfo {
                name: "router01".to_string(),
                kind: NodeKind::VirtualMachine,
                model: NodeModel::CiscoCat8000v,
                status: NodeState::Running,
                ip_address: Some("172.31.0.12".to_string()),
                ssh_port: Some(22),
            },
        ];

        let table = render_nodes_table(&nodes, "a1b2c3d4");
        println!("\n{table}");

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
            kind: NodeKind::Container,
            model: NodeModel::AlpineLinux,
            status: NodeState::Starting,
            ip_address: None,
            ssh_port: None,
        }];

        let table = render_nodes_table(&nodes, "a1b2c3d4");

        // Check that missing data is represented with "-"
        assert!(table.contains("dev01"));
        assert!(table.contains("alpine_linux"));
        // The table should have "-" for missing IP and connection
        assert!(table.matches('-').count() >= 2);
    }

    #[test]
    fn test_render_empty_nodes() {
        let nodes: Vec<NodeInfo> = vec![];
        let table = render_nodes_table(&nodes, "a1b2c3d4");

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
            cpu_count: None,
            memory: None,
        }];

        let table = render_devices_table(&devices, "a1b2c3d4");

        // Check that table contains expected data
        assert!(table.contains("router01.a1b2c3d4"));
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
                cpu_count: None,
                memory: None,
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
                cpu_count: None,
                memory: None,
            },
        ];

        let table = render_devices_table(&devices, "a1b2c3d4");
        println!("\n{table}");

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
    fn test_render_device_cpu_and_memory() {
        let devices = vec![
            DeviceInfo {
                name: "dev01".to_string(),
                model: NodeModel::UbuntuLinux,
                kind: NodeKind::VirtualMachine,
                state: NodeState::Running,
                mgmt_ipv4: "172.31.0.21".to_string(),
                mgmt_ipv6: None,
                vnc_port: Some(5901),
                disks: vec![],
                cpu_count: Some(16),
                memory: Some(32768),
            },
            DeviceInfo {
                name: "db01".to_string(),
                model: NodeModel::MongoDb,
                kind: NodeKind::Container,
                state: NodeState::Running,
                mgmt_ipv4: "172.31.0.22".to_string(),
                mgmt_ipv6: None,
                vnc_port: None,
                disks: vec![],
                cpu_count: None,
                memory: None,
            },
        ];

        let table = render_devices_table(&devices, "a1b2c3d4");
        assert!(table.contains("CPU"), "{table}");
        assert!(table.contains("Memory (MB)"), "{table}");
        let vm = table.lines().find(|line| line.contains("dev01")).unwrap();
        assert!(
            vm.contains("│ 16 ") && vm.contains("│ 32768 ") && vm.contains("│ 5901 "),
            "{vm}"
        );
        // Containers are not limited.
        let container = table.lines().find(|line| line.contains("db01")).unwrap();
        assert_eq!(container.matches("│ - ").count(), 5, "{container}");
    }

    #[test]
    fn test_render_device_missing_ip() {
        let devices = vec![DeviceInfo {
            name: "container01".to_string(),
            model: NodeModel::AlpineLinux,
            kind: NodeKind::Container,
            state: NodeState::Stopped,
            mgmt_ipv4: String::new(),
            mgmt_ipv6: None,
            vnc_port: None,
            disks: vec![],
            cpu_count: None,
            memory: None,
        }];

        let table = render_devices_table(&devices, "a1b2c3d4");

        // Check that missing data is represented with "-"
        assert!(table.contains("container01"));
        assert!(table.contains("alpine_linux"));
        assert!(table.contains("stopped")); // state = Stopped
        assert!(table.contains('-')); // empty mgmt_ipv4
    }

    #[test]
    fn test_render_empty_devices() {
        let devices: Vec<DeviceInfo> = vec![];
        let table = render_devices_table(&devices, "a1b2c3d4");

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
        let lab_info = LabInfo {
            ssh_usernames: Vec::default(),
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
            description: None,
            client_host: None,
            client_folder: None,
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
        println!("\n{table}");
        assert!(!table.contains("Client"), "{table}");
    }

    #[test]
    fn lab_info_table_shows_where_the_lab_was_started() {
        let lab_info = LabInfo {
            ssh_usernames: Vec::default(),
            id: "039ab286".to_string(),
            name: "core".to_string(),
            user: "alice".to_string(),
            ipv4_network: "172.31.0.0/24".parse().unwrap(),
            ipv4_gateway: "172.31.0.1".parse().unwrap(),
            ipv4_router: "172.31.0.2".parse().unwrap(),
            loopback_network: "127.127.1.0/24".parse().unwrap(),
            ipv6_network: None,
            ipv6_gateway: None,
            ipv6_router: None,
            description: Some("Practice BGP between two routers.".to_string()),
            client_host: Some("dev01".to_string()),
            client_folder: Some("/home/alice/labs/core".to_string()),
        };

        let table = render_lab_info_table(&lab_info);

        assert!(table.contains("Description"), "{table}");
        assert!(
            table.contains("Practice BGP between two routers."),
            "{table}"
        );
        assert!(table.contains("Client Host"), "{table}");
        assert!(table.contains("dev01"), "{table}");
        assert!(table.contains("Client Folder"), "{table}");
        assert!(table.contains("/home/alice/labs/core"), "{table}");
    }
}
