//! Lab tables: a lab's info, nodes, devices, links, bridges, provisioning results and
//! Tailscale gateway.

use shared::data::{
    BridgeInfo, DeviceInfo, LabInfo, LinkInfo, LinkState, NodeInfo, ProvisionResult, TailnetStatus,
};
use shared::util::{TableLayout, render_table};
use tracing::instrument;

const NODE_HEADERS: [&str; 5] = ["Node", "Mgmt IP", "Connection", "Node Model", "Kind"];
const DEVICE_HEADERS: [&str; 9] = [
    "Device",
    "Model",
    "Kind",
    "State",
    "Mgmt IPv4",
    "Mgmt IPv6",
    "CPU",
    "Memory (MB)",
    "Disks",
];
const LINK_HEADERS: [&str; 6] = [
    "Node A",
    "Interface A",
    "Node B",
    "Interface B",
    "Type",
    "State",
];
const BRIDGE_HEADERS: [&str; 3] = ["Bridge", "Network", "Connected Nodes"];
const PROVISION_HEADERS: [&str; 3] = ["Node", "Status", "Detail"];
const TAILNET_HEADERS: [&str; 2] = ["Tailscale", "Details"];
/// The lab info table has no header row: its first column names each row.
const LAB_INFO_HEADERS: [&str; 2] = ["Property", "Value"];
/// The widest the Tailscale table may be, borders and padding included.
const TAILNET_WIDTH: usize = 80;

/// Renders a table of nodes with their management IP, connection info, model and kind. Each
/// node is named `<name>.<lab_id>`.
#[must_use]
pub fn render_nodes_table(nodes: &[NodeInfo], lab_id: &str) -> String {
    let rows: Vec<[String; 5]> = nodes.iter().map(|node| node_cells(node, lab_id)).collect();
    render_table(
        NODE_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Nodes"),
            ..TableLayout::default()
        },
    )
}

fn node_cells(node: &NodeInfo, lab_id: &str) -> [String; 5] {
    let connection = match (&node.ip_address, node.ssh_port) {
        (Some(ip), Some(port)) => format!("{ip}:{port}"),
        _ => "-".to_string(),
    };
    [
        format!("{}.{lab_id}", node.name),
        node.ip_address.as_deref().unwrap_or("-").to_string(),
        connection,
        node.model.to_string(),
        node.kind.to_string(),
    ]
}

/// Renders a table of devices with their model, kind, state, management IPs, vCPUs, memory
/// and disks. Each device is named `<name>.<lab_id>`.
#[must_use]
pub fn render_devices_table(devices: &[DeviceInfo], lab_id: &str) -> String {
    let rows: Vec<[String; 9]> = devices
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

fn device_cells(device: &DeviceInfo, lab_id: &str) -> [String; 9] {
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
    // A container without a limit shows "-".
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
        disks,
    ]
}

/// Renders a table of point-to-point links between nodes
#[must_use]
pub fn render_links_table(links: &[LinkInfo]) -> String {
    let rows: Vec<[String; 6]> = links.iter().map(link_cells).collect();
    render_table(
        LINK_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Links"),
            ..TableLayout::default()
        },
    )
}

fn link_cells(link: &LinkInfo) -> [String; 6] {
    [
        link.node_a_name.clone(),
        link.int_a.clone(),
        link.node_b_name.clone(),
        link.int_b.clone(),
        link.kind.to_string(),
        link_state_cell(link.state.as_ref()),
    ]
}

/// A link's state as the table shows it. A link the server has not checked since it started
/// is "not checked".
fn link_state_cell(state: Option<&LinkState>) -> String {
    match state {
        None => "not checked".to_string(),
        Some(LinkState::Attached) => "attached".to_string(),
        Some(LinkState::PeerDown { peer }) => format!("peer down: {peer}"),
        Some(LinkState::Failed { reason }) => format!("failed: {reason}"),
    }
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

    use super::*;
    use shared::data::{BridgeKind, NodeKind, NodeModel, NodeState, ProvisionStatus};

    #[test]
    fn links_table_shows_each_link_state() {
        let link = |node_b: &str, state| LinkInfo {
            node_a_name: "vm1".to_string(),
            int_a: "eth1".to_string(),
            node_b_name: node_b.to_string(),
            int_b: "eth1".to_string(),
            kind: BridgeKind::P2p,
            state,
            index: None,
        };
        let expected = "\
┌──────────────────────────────────────────────────────────────────────────────────────────┐
│                                          Links                                           │
├────────┬─────────────┬────────┬─────────────┬──────┬─────────────────────────────────────┤
│ Node A │ Interface A │ Node B │ Interface B │ Type │ State                               │
├────────┼─────────────┼────────┼─────────────┼──────┼─────────────────────────────────────┤
│ vm1    │ eth1        │ vm2    │ eth1        │ p2p  │ attached                            │
├────────┼─────────────┼────────┼─────────────┼──────┼─────────────────────────────────────┤
│ vm1    │ eth1        │ ctr1   │ eth1        │ p2p  │ peer down: ctr1                     │
├────────┼─────────────┼────────┼─────────────┼──────┼─────────────────────────────────────┤
│ vm1    │ eth1        │ uk1    │ eth1        │ p2p  │ failed: its end tpb2 does not exist │
├────────┼─────────────┼────────┼─────────────┼──────┼─────────────────────────────────────┤
│ vm1    │ eth1        │ vm3    │ eth1        │ p2p  │ not checked                         │
└────────┴─────────────┴────────┴─────────────┴──────┴─────────────────────────────────────┘";
        assert_eq!(
            render_links_table(&[
                link("vm2", Some(LinkState::Attached)),
                link(
                    "ctr1",
                    Some(LinkState::PeerDown {
                        peer: "ctr1".to_string(),
                    }),
                ),
                link(
                    "uk1",
                    Some(LinkState::Failed {
                        reason: "its end tpb2 does not exist".to_string(),
                    }),
                ),
                link("vm3", None),
            ]),
            expected
        );
    }

    #[test]
    fn provision_report_003_the_table_shows_each_node_with_its_status_and_failure_detail() {
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
        let expected = "\
┌────────────────────────────────────────────────────────────────┐
│                          Provisioning                          │
├────────────────┬─────────────┬─────────────────────────────────┤
│ Node           │ Status      │ Detail                          │
├────────────────┼─────────────┼─────────────────────────────────┤
│ dev01.abcd1234 │ provisioned │                                 │
├────────────────┼─────────────┼─────────────────────────────────┤
│ dev02.abcd1234 │ failed      │ exit status 3 on dev02.abcd1234 │
└────────────────┴─────────────┴─────────────────────────────────┘";
        assert_eq!(render_provision_table(&results, "abcd1234"), expected);
    }

    #[test]
    fn tailnet_table_renders_approved_status_without_warnings() {
        let status = TailnetStatus::from_daemon(
            r#"{"BackendState":"Running","TailscaleIPs":["100.64.0.1"],"Self":{"AllowedIPs":["172.31.3.0/24"]}}"#,
            vec!["172.31.3.0/24".into()],
        )
        .unwrap();

        let expected = "\
┌─────────────────────┬───────────────┐
│ Tailscale           │ Details       │
├─────────────────────┼───────────────┤
│ Gateway             │ Connected     │
├─────────────────────┼───────────────┤
│ Route approval      │ Approved      │
├─────────────────────┼───────────────┤
│ Management routes   │ 172.31.3.0/24 │
├─────────────────────┼───────────────┤
│ Tailscale addresses │ 100.64.0.1    │
├─────────────────────┼───────────────┤
│ Warnings            │               │
└─────────────────────┴───────────────┘";
        assert_eq!(render_tailnet_table(&status), expected);
    }

    #[test]
    fn the_nodes_table_names_each_nodes_kind() {
        let nodes = [
            NodeInfo {
                name: "veos1".to_string(),
                kind: NodeKind::VirtualMachine,
                model: NodeModel::AristaEos,
                status: NodeState::Running,
                ip_address: Some("172.31.0.11".to_string()),
                ssh_port: Some(22),
            },
            NodeInfo {
                name: "ceos1".to_string(),
                kind: NodeKind::Container,
                model: NodeModel::AristaEos,
                status: NodeState::Running,
                ip_address: None,
                ssh_port: None,
            },
        ];
        let expected = "\
┌──────────────────────────────────────────────────────────────────────────────┐
│                                    Nodes                                     │
├────────────────┬─────────────┬────────────────┬────────────┬─────────────────┤
│ Node           │ Mgmt IP     │ Connection     │ Node Model │ Kind            │
├────────────────┼─────────────┼────────────────┼────────────┼─────────────────┤
│ veos1.a1b2c3d4 │ 172.31.0.11 │ 172.31.0.11:22 │ arista_eos │ virtual_machine │
├────────────────┼─────────────┼────────────────┼────────────┼─────────────────┤
│ ceos1.a1b2c3d4 │ -           │ -              │ arista_eos │ container       │
└────────────────┴─────────────┴────────────────┴────────────┴─────────────────┘";
        assert_eq!(render_nodes_table(&nodes, "a1b2c3d4"), expected);
    }

    #[test]
    fn a_device_row_shows_each_of_its_values() {
        let devices = vec![DeviceInfo {
            name: "router01".to_string(),
            model: NodeModel::CiscoCat8000,
            kind: NodeKind::VirtualMachine,
            state: NodeState::Running,
            mgmt_ipv4: "172.31.0.11".to_string(),
            mgmt_ipv6: None,
            disks: vec!["/var/lib/sherpa/labs/test/router01.qcow2".to_string()],
            cpu_count: None,
            memory: None,
        }];

        let expected = "\
┌────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                                                      Active Nodes                                                                      │
├───────────────────┬───────────────┬─────────────────┬─────────┬─────────────┬───────────┬─────┬─────────────┬──────────────────────────────────────────┤
│ Device            │ Model         │ Kind            │ State   │ Mgmt IPv4   │ Mgmt IPv6 │ CPU │ Memory (MB) │ Disks                                    │
├───────────────────┼───────────────┼─────────────────┼─────────┼─────────────┼───────────┼─────┼─────────────┼──────────────────────────────────────────┤
│ router01.a1b2c3d4 │ cisco_cat8000 │ virtual_machine │ running │ 172.31.0.11 │ -         │ -   │ -           │ /var/lib/sherpa/labs/test/router01.qcow2 │
└───────────────────┴───────────────┴─────────────────┴─────────┴─────────────┴───────────┴─────┴─────────────┴──────────────────────────────────────────┘";
        assert_eq!(render_devices_table(&devices, "a1b2c3d4"), expected);
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
            ipv6_network: None,
            ipv6_gateway: None,
            ipv6_router: None,
            description: Some("Practice BGP between two routers.".to_string()),
            client_host: Some("dev01".to_string()),
            client_folder: Some("/home/alice/labs/core".to_string()),
        };

        let expected = "\
┌────────────────────────────────────────────────────────┐
│                        Lab Info                        │
├────────────────────┬───────────────────────────────────┤
│ ID                 │ 039ab286                          │
├────────────────────┼───────────────────────────────────┤
│ Name               │ core                              │
├────────────────────┼───────────────────────────────────┤
│ Description        │ Practice BGP between two routers. │
├────────────────────┼───────────────────────────────────┤
│ User               │ alice                             │
├────────────────────┼───────────────────────────────────┤
│ Management Network │ 172.31.0.0/24                     │
├────────────────────┼───────────────────────────────────┤
│ IPv4 Gateway       │ 172.31.0.1                        │
├────────────────────┼───────────────────────────────────┤
│ IPv4 Router        │ 172.31.0.2                        │
├────────────────────┼───────────────────────────────────┤
│ Client Host        │ dev01                             │
├────────────────────┼───────────────────────────────────┤
│ Client Folder      │ /home/alice/labs/core             │
└────────────────────┴───────────────────────────────────┘";
        assert_eq!(render_lab_info_table(&lab_info), expected);
    }
}
