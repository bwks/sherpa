use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use anyhow::{Context, Result};
use ipnet::{Ipv4Net, Ipv6Net};
use schemars::JsonSchema;
use serde_derive::{Deserialize, Serialize};

use super::{BridgeKind, DbNode, NodeKind, NodeModel, NodeState};

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
    pub p2p: bool,
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

/// Minimal lab identity (name + id) used by CLI commands that don't need full LabInfo.
#[derive(Debug, Clone)]
pub struct LabIdentity {
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LabInfo {
    pub id: String,
    pub name: String,
    pub user: String,
    #[schemars(with = "String")]
    pub ipv4_network: Ipv4Net,
    #[schemars(with = "String")]
    pub ipv4_gateway: Ipv4Addr,
    #[schemars(with = "String")]
    pub ipv4_router: Ipv4Addr,
    #[schemars(with = "String")]
    pub loopback_network: Ipv4Net,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub ipv6_network: Option<Ipv6Net>,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub ipv6_gateway: Option<Ipv6Addr>,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub ipv6_router: Option<Ipv6Addr>,
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
    /// Tap device name for node_a side (P2p links only).
    pub tap_a: String,
    /// Tap device name for node_b side (P2p links only).
    pub tap_b: String,
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

/// Lab status enumeration for displaying current state.
///
/// Derived from the aggregate state of all nodes in the lab.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LabStatus {
    /// All nodes are running
    Running,
    /// All nodes are stopped
    Stopped,
    /// Mix of running, stopped, or other states
    Partial,
    /// At least one node is starting, none failed
    Starting,
    /// At least one node has failed
    Failed,
    /// Lab has no nodes
    Empty,
    /// Status cannot be determined
    Unknown,
}

impl LabStatus {
    /// Derive the aggregate lab status from the states of its nodes.
    pub fn derive(states: &[NodeState]) -> Self {
        if states.is_empty() {
            return LabStatus::Empty;
        }

        let all_running = states.iter().all(|s| *s == NodeState::Running);
        let all_stopped = states.iter().all(|s| *s == NodeState::Stopped);
        let any_failed = states.contains(&NodeState::Failed);
        let any_starting = states.contains(&NodeState::Starting);

        if all_running {
            LabStatus::Running
        } else if all_stopped {
            LabStatus::Stopped
        } else if any_failed {
            LabStatus::Failed
        } else if any_starting {
            LabStatus::Starting
        } else {
            LabStatus::Partial
        }
    }
}

impl fmt::Display for LabStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabStatus::Running => write!(f, "running"),
            LabStatus::Stopped => write!(f, "stopped"),
            LabStatus::Partial => write!(f, "partial"),
            LabStatus::Starting => write!(f, "starting"),
            LabStatus::Failed => write!(f, "failed"),
            LabStatus::Empty => write!(f, "empty"),
            LabStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// Summary information about a lab for list views
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LabSummary {
    /// Lab ID (business key)
    pub id: String,
    /// Human-readable lab name
    pub name: String,
    /// Number of nodes in the lab
    pub node_count: usize,
    /// Current status of the lab
    pub status: LabStatus,
}

/// Response for listing labs
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListLabsResponse {
    /// List of lab summaries
    pub labs: Vec<LabSummary>,
    /// Total number of labs
    pub total: usize,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn make_lab_info() -> LabInfo {
        LabInfo {
            id: "lab-001".to_string(),
            name: "test-lab".to_string(),
            user: "alice".to_string(),
            ipv4_network: "192.168.100.0/24".parse().unwrap(),
            ipv4_gateway: "192.168.100.1".parse().unwrap(),
            ipv4_router: "192.168.100.2".parse().unwrap(),
            loopback_network: "127.127.127.0/24".parse().unwrap(),
            ipv6_network: None,
            ipv6_gateway: None,
            ipv6_router: None,
        }
    }

    #[test]
    fn test_lab_info_display_and_from_str_round_trip() {
        let lab = make_lab_info();
        let s = lab.to_string();
        let parsed = LabInfo::from_str(&s).unwrap();
        assert_eq!(parsed.id, "lab-001");
        assert_eq!(parsed.name, "test-lab");
        assert_eq!(parsed.user, "alice");
        assert_eq!(
            parsed.ipv4_gateway,
            "192.168.100.1".parse::<std::net::Ipv4Addr>().unwrap()
        );
    }

    #[test]
    fn test_lab_info_from_str_invalid_returns_err() {
        assert!(LabInfo::from_str("not valid toml").is_err());
    }

    #[test]
    fn test_lab_status_display() {
        assert_eq!(LabStatus::Running.to_string(), "running");
        assert_eq!(LabStatus::Stopped.to_string(), "stopped");
        assert_eq!(LabStatus::Partial.to_string(), "partial");
        assert_eq!(LabStatus::Starting.to_string(), "starting");
        assert_eq!(LabStatus::Failed.to_string(), "failed");
        assert_eq!(LabStatus::Empty.to_string(), "empty");
        assert_eq!(LabStatus::Unknown.to_string(), "unknown");
    }

    #[test]
    fn test_lab_status_serde_round_trip() {
        let variants = vec![
            LabStatus::Running,
            LabStatus::Stopped,
            LabStatus::Partial,
            LabStatus::Starting,
            LabStatus::Failed,
            LabStatus::Empty,
            LabStatus::Unknown,
        ];
        for status in variants {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: LabStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn test_lab_status_derive_empty() {
        assert_eq!(LabStatus::derive(&[]), LabStatus::Empty);
    }

    #[test]
    fn test_lab_status_derive_all_running() {
        let states = vec![NodeState::Running, NodeState::Running, NodeState::Running];
        assert_eq!(LabStatus::derive(&states), LabStatus::Running);
    }

    #[test]
    fn test_lab_status_derive_all_stopped() {
        let states = vec![NodeState::Stopped, NodeState::Stopped];
        assert_eq!(LabStatus::derive(&states), LabStatus::Stopped);
    }

    #[test]
    fn test_lab_status_derive_any_failed() {
        let states = vec![NodeState::Running, NodeState::Failed];
        assert_eq!(LabStatus::derive(&states), LabStatus::Failed);
    }

    #[test]
    fn test_lab_status_derive_any_starting() {
        let states = vec![NodeState::Running, NodeState::Starting];
        assert_eq!(LabStatus::derive(&states), LabStatus::Starting);
    }

    #[test]
    fn test_lab_status_derive_partial() {
        let states = vec![NodeState::Running, NodeState::Stopped];
        assert_eq!(LabStatus::derive(&states), LabStatus::Partial);
    }

    #[test]
    fn test_lab_status_derive_mixed_unknown_and_running() {
        let states = vec![NodeState::Running, NodeState::Unknown];
        assert_eq!(LabStatus::derive(&states), LabStatus::Partial);
    }
}
