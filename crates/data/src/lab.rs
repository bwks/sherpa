use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

use anyhow::{Context, Result};
use ipnet::Ipv4Net;
use serde_derive::{Deserialize, Serialize};

use super::{BridgeKind, DbNode, NodeKind, NodeModel};

#[derive(Clone, Serialize, Deserialize)]
pub struct LabInfo {
    pub id: String,
    pub name: String,
    pub user: String,
    pub ipv4_network: Ipv4Net,
    pub ipv4_gateway: Ipv4Addr,
    pub ipv4_router: Ipv4Addr,
}
impl fmt::Display for LabInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let toml_string = toml::to_string_pretty(self).map_err(|_| fmt::Error)?;
        write!(f, "{}", toml_string)
    }
}
impl FromStr for LabInfo {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        toml::from_str(s).context("Failed to parse LabInfo from TOML")
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LabNodeData {
    pub name: String,
    pub model: NodeModel,
    pub kind: NodeKind,
    pub index: u16,
    pub record: DbNode,
}

#[derive(Clone)]
pub struct NodeSetupData {
    pub name: String,
    pub index: u16,
    pub isolated_network: Option<String>,
    pub reserved_network: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LabLinkData {
    pub index: u16,
    pub kind: BridgeKind,
    pub node_a: DbNode,
    pub node_b: DbNode,
    pub int_a: String,
    pub int_b: String,
    pub bridge_a: String,
    pub bridge_b: String,
    pub veth_a: String,
    pub veth_b: String,
}
