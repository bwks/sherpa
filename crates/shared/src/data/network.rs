use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

use ipnet::{Ipv4Net, Ipv6Net};
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

#[derive(Clone, Debug, Deserialize, Default, Serialize, EnumIter, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BridgeKind {
    P2p,
    #[default]
    P2pBridge,
    P2pUdp,
    P2pVeth,
    SharedBridge,
}
impl fmt::Display for BridgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeKind::P2p => write!(f, "p2p"),
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
impl_surreal_value_for_enum!(BridgeKind);

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
#[derive(Clone)]
pub struct NetworkV6 {
    pub prefix: Ipv6Net,
    pub first: Ipv6Addr,
    pub last: Ipv6Addr,
    pub boot_server: Ipv6Addr,
    pub network: Ipv6Addr,
    pub prefix_length: u8,
}

#[derive(Clone)]
pub struct SherpaNetwork {
    pub v4: NetworkV4,
    pub v6: Option<NetworkV6>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bridge_kind_display() {
        assert_eq!(BridgeKind::P2p.to_string(), "p2p");
        assert_eq!(BridgeKind::P2pBridge.to_string(), "p2p_bridge");
        assert_eq!(BridgeKind::P2pUdp.to_string(), "p2p_udp");
        assert_eq!(BridgeKind::P2pVeth.to_string(), "p2p_veth");
        assert_eq!(BridgeKind::SharedBridge.to_string(), "shared_bridge");
    }

    #[test]
    fn test_bridge_kind_to_vec() {
        let kinds = BridgeKind::to_vec();
        assert_eq!(kinds.len(), 5);
    }

    #[test]
    fn test_bridge_kind_serde_roundtrip() {
        let kind = BridgeKind::P2pVeth;
        let json = serde_json::to_string(&kind).expect("serializes");
        assert_eq!(json, "\"p2p_veth\"");
        let back: BridgeKind = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, BridgeKind::P2pVeth);
    }

    #[test]
    fn test_bridge_kind_default() {
        let kind = BridgeKind::default();
        assert_eq!(kind, BridgeKind::P2pBridge);
    }
}
