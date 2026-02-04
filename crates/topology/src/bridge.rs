use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use data::NodeModel;
use util::split_node_int;

/// Bridge connection in manifest format
/// Expected format: "node_name::interface_name"
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BridgeConnection {
    pub node_name: String,
    pub interface_name: String,
}

impl BridgeConnection {
    /// Parse a bridge connection string in format "node_name::interface_name"
    pub fn parse(connection_str: &str) -> Result<Self> {
        let conn_str = connection_str.to_string();
        let (node_name, interface_name) = split_node_int(&conn_str)?;
        Ok(Self {
            node_name,
            interface_name,
        })
    }
}

/// Bridge with parsed connections
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Bridge {
    pub connections: Vec<String>,
}

impl Bridge {
    /// Parse all connections in this bridge
    pub fn parse_connections(&self) -> Result<Vec<BridgeConnection>> {
        self.connections
            .iter()
            .map(|conn| BridgeConnection::parse(conn))
            .collect()
    }
}

/// Expanded bridge connection with node and interface details
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BridgeConnectionExpanded {
    pub node_name: String,
    pub interface_name: String,
}

/// Detailed bridge with resolved node models and interface indices
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BridgeDetailed {
    pub connections: Vec<BridgeConnectionDetailed>,
    pub bridge_index: u16,
}

/// Detailed bridge connection with all resolved information
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BridgeConnectionDetailed {
    pub node_name: String,
    pub node_model: NodeModel,
    pub interface_name: String,
    pub interface_index: u8,
}
