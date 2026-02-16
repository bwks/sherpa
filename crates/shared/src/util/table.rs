use tabled::{Table, Tabled, settings::Style};

use crate::data::NodeInfo;

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

    Table::new(rows).with(Style::modern()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::node::{NodeModel, NodeState};

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
                model: NodeModel::CiscoCat8000V,
                status: NodeState::Running,
                ip_address: Some("172.31.0.12".to_string()),
                ssh_port: Some(22),
            },
        ];

        let table = render_nodes_table(&nodes);

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
}
