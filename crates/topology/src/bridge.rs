use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use shared::data::NodeModel;
use shared::util::split_node_int;

/// Bridge connection in manifest format
/// Expected format: "node_name::interface_name"
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BridgeLink {
    pub node: String,
    pub interface: String,
}

impl BridgeLink {
    /// Parse a bridge connection string in format "node_name::interface_name"
    pub fn parse(connection_str: &str) -> Result<Self> {
        let (node, interface) = split_node_int(connection_str)?;
        Ok(Self { node, interface })
    }
}

/// Bridge with parsed connections
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Bridge {
    pub name: String,
    pub links: Vec<String>,
}

impl Bridge {
    /// Parse all links in this bridge
    pub fn parse_links(&self) -> Result<BridgeExpanded> {
        let bridge_links = self
            .links
            .iter()
            .map(|conn| BridgeLink::parse(conn))
            .collect::<Result<Vec<BridgeLink>>>()?;

        Ok(BridgeExpanded {
            name: self.name.clone(),
            links: bridge_links,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BridgeExpanded {
    pub name: String,
    pub links: Vec<BridgeLink>,
}

/// Expanded bridge connection with node and interface details
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BridgeLinkExpanded {
    pub node_name: String,
    pub interface_idx: u8,
}

/// Detailed bridge with resolved node models and interface indices
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BridgeDetailed {
    pub manifest_name: String,
    pub bridge_name: String,
    pub libvirt_name: String,
    pub index: u16,
    pub links: Vec<BridgeLinkDetailed>,
}

/// Detailed bridge connection with all resolved information
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BridgeLinkDetailed {
    pub node_name: String,
    pub node_model: NodeModel,
    pub interface_name: String,
    pub interface_index: u8,
}
