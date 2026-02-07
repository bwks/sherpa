use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

use anyhow::{Context, Result};
use ipnet::Ipv4Net;
use serde_derive::{Deserialize, Serialize};

use super::{BridgeKind, DbNode, NodeKind, NodeModel};

#[derive(Clone, Debug)]
pub enum PeerSide {
    A,
    B,
}

#[derive(Clone, Debug)]
pub struct PeerInterface {
    pub link_index: u16,
    pub this_node: String,
    pub this_node_index: u16,
    pub this_interface: String,
    pub this_interface_index: u8,
    pub this_side: PeerSide,
    pub peer_node: String,
    pub peer_node_index: u16,
    pub peer_interface: String,
    pub peer_interface_index: u8,
    pub peer_side: PeerSide,
}

#[derive(Clone, Debug)]
pub struct BridgeInterface {
    pub name: String,
}

#[derive(Clone, Debug)]
pub enum InterfaceState {
    Enabled,
    Disabled,
}

#[derive(Clone, Debug)]
pub enum NodeInterface {
    Management,
    Reserved,
    Disabled,
    Peer(PeerInterface),
    Bridge(BridgeInterface),
}

#[derive(Clone, Debug)]
pub struct InterfaceData {
    pub name: String,
    pub index: u8,
    pub state: InterfaceState,
    pub data: NodeInterface,
}

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

#[derive(Clone, Debug)]
pub struct NodeSetupData {
    pub name: String,
    pub index: u16,
    pub management_network: String,
    pub isolated_network: Option<LabIsolatedNetwork>,
    pub reserved_network: Option<LabReservedNetwork>,
    pub interfaces: Vec<InterfaceData>,
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

#[derive(Clone, Serialize, Deserialize)]
pub struct LabBridgeData {
    pub index: u16,
    pub bridge_name: String,
    pub network_name: String,
    pub connections: Vec<BridgeConnection>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BridgeConnection {
    pub node_record: DbNode,
    pub interface_name: String,
    pub interface_index: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LabIsolatedNetwork {
    pub network_name: String,
    pub bridge_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LabReservedNetwork {
    pub network_name: String,
    pub bridge_name: String,
}
