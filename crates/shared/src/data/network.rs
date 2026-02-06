use std::fmt;
use std::net::Ipv4Addr;

use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

#[derive(Clone, Debug, Deserialize, Default, Serialize, EnumIter, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BridgeKind {
    #[default]
    P2pBridge,
    P2pUdp,
    P2pVeth,
    SharedBridge,
}
impl fmt::Display for BridgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeKind::P2pBridge => write!(f, "p2p_bridge"),
            BridgeKind::P2pUdp => write!(f, "p2p_udp"),
            BridgeKind::P2pVeth => write!(f, "p2p_veth"),
            BridgeKind::SharedBridge => write!(f, "shared_bridge"),
        }
    }
}
impl BridgeKind {
    pub fn to_vec() -> Vec<BridgeKind> {
        BridgeKind::iter().collect()
    }
}

#[derive(Clone)]
pub struct NetworkV4 {
    pub prefix: Ipv4Net,
    pub first: Ipv4Addr,
    pub last: Ipv4Addr,
    pub boot_server: Ipv4Addr,
    pub network: Ipv4Addr,
    pub subnet_mask: Ipv4Addr,
    pub hostmask: Ipv4Addr,
    pub prefix_length: u8,
}
// pub struct NetworkV6;

#[derive(Clone)]
pub struct SherpaNetwork {
    pub v4: NetworkV4,
    // v6: SherpaManagementV6,
}
