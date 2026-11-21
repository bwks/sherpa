use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use anyhow::{Context, Result};
use ipnet::{Ipv4Net, Ipv6Net};
use schemars::JsonSchema;
use serde_derive::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;
use tracing::instrument;

use super::{BridgeKind, DbNode, LabPhase, ModelKind, NodeState};

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

/// Minimal lab identity (name + id) used by CLI commands that don't need full `LabInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabIdentity {
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LabInfo {
    /// Personal SSH accounts actually provisioned for lab nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ssh_usernames: Vec<NodeSshUsername>,
    pub id: String,
    pub name: String,
    pub user: String,
    #[schemars(with = "String")]
    pub ipv4_network: Ipv4Net,
    #[schemars(with = "String")]
    pub ipv4_gateway: Ipv4Addr,
    #[schemars(with = "String")]
    pub ipv4_router: Ipv4Addr,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub ipv6_network: Option<Ipv6Net>,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub ipv6_gateway: Option<Ipv6Addr>,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub ipv6_router: Option<Ipv6Addr>,
    /// The lab's description, from its manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The hostname of the machine `sherpa up` ran on. Labs started from the web UI or the API
    /// have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_host: Option<String>,
    /// The folder `sherpa up` ran in, on that machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_folder: Option<String>,
}

/// Saved personal SSH identity of a provisioned node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NodeSshUsername {
    pub node_name: String,
    pub username: String,
}

impl LabInfo {
    #[instrument(level = "debug", skip(self))]
    pub fn ssh_username(&self, node_name: &str) -> Option<&str> {
        self.ssh_usernames
            .iter()
            .find(|entry| entry.node_name == node_name)
            .map(|entry| entry.username.as_str())
    }

    /// Replace one node's SSH identity while preserving other nodes' identities.
    #[instrument(level = "debug", skip(self))]
    pub fn set_ssh_username(&mut self, node_name: &str, username: Option<&str>) {
        self.ssh_usernames
            .retain(|entry| entry.node_name != node_name);
        if let Some(username) = username {
            self.ssh_usernames.push(NodeSshUsername {
                node_name: node_name.to_owned(),
                username: username.to_owned(),
            });
        }
    }
}
impl fmt::Display for LabInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let toml_string = toml::to_string_pretty(self).map_err(|_| fmt::Error)?;
        write!(f, "{toml_string}")
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
    pub model_kind: ModelKind,
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
    /// Tap device name for `node_a` side (P2p links only).
    pub tap_a: String,
    /// Tap device name for `node_b` side (P2p links only).
    pub tap_b: String,
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
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, EnumIter,
)]
#[serde(rename_all = "lowercase")]
pub enum LabState {
    /// All nodes are running
    Running,
    /// All nodes are stopped
    Stopped,
    /// Mix of running, stopped, or other states
    Partial,
    /// At least one node is being built or starting, none failed or missing
    Starting,
    /// At least one node has failed or is missing
    Failed,
    /// Lab has no nodes
    Empty,
    /// Status cannot be determined, or one this build does not know, such as one sent by a
    /// newer server.
    #[default]
    #[serde(other)]
    Unknown,
}

impl LabState {
    /// Returns all variants as a vector (used by DB schema generation).
    #[must_use]
    pub fn to_vec() -> Vec<LabState> {
        LabState::iter().collect()
    }

    /// Derive the aggregate lab status from the states of its nodes.
    #[must_use]
    pub fn derive(states: &[NodeState]) -> Self {
        if states.is_empty() {
            return LabState::Empty;
        }

        let all_running = states.iter().all(|s| *s == NodeState::Running);
        let all_stopped = states.iter().all(|s| *s == NodeState::Stopped);
        let any_failed = states
            .iter()
            .any(|s| matches!(s, NodeState::Failed | NodeState::Missing));
        let any_starting = states
            .iter()
            .any(|s| matches!(s, NodeState::Starting | NodeState::Building));

        if all_running {
            LabState::Running
        } else if all_stopped {
            LabState::Stopped
        } else if any_failed {
            LabState::Failed
        } else if any_starting {
            LabState::Starting
        } else {
            LabState::Partial
        }
    }
}

impl fmt::Display for LabState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabState::Running => write!(f, "running"),
            LabState::Stopped => write!(f, "stopped"),
            LabState::Partial => write!(f, "partial"),
            LabState::Starting => write!(f, "starting"),
            LabState::Failed => write!(f, "failed"),
            LabState::Empty => write!(f, "empty"),
            LabState::Unknown => write!(f, "unknown"),
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
    /// The lab's description, from its manifest
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Number of nodes in the lab
    pub node_count: usize,
    /// The lab's status, derived from its nodes
    pub status: LabState,
    /// Which operation owns the lab, if any. While it is not `ready`, the phase is what to
    /// show instead of the status.
    #[serde(default)]
    pub phase: LabPhase,
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
    use super::*;

    fn make_lab_info() -> LabInfo {
        LabInfo {
            ssh_usernames: Vec::new(),
            id: "lab-001".to_string(),
            name: "test-lab".to_string(),
            user: "alice".to_string(),
            ipv4_network: "192.168.100.0/24".parse().unwrap(),
            ipv4_gateway: "192.168.100.1".parse().unwrap(),
            ipv4_router: "192.168.100.2".parse().unwrap(),
            ipv6_network: None,
            ipv6_gateway: None,
            ipv6_router: None,
            description: Some("Practice BGP.\nTwo routers.".to_string()),
            client_host: Some("dev01".to_string()),
            client_folder: Some("/home/alice/labs/core".to_string()),
        }
    }

    #[test]
    fn updating_one_nodes_ssh_identity_preserves_other_nodes() {
        let mut lab = make_lab_info();
        lab.set_ssh_username("vm1", Some("alice"));
        lab.set_ssh_username("vm2", Some("bob"));
        lab.set_ssh_username("vm1", Some("carol"));
        assert_eq!(lab.ssh_username("vm1"), Some("carol"));
        assert_eq!(lab.ssh_username("vm2"), Some("bob"));
        assert_eq!(lab.ssh_usernames.len(), 2);

        lab.set_ssh_username("vm1", None);
        assert_eq!(lab.ssh_username("vm1"), None);
        assert_eq!(lab.ssh_username("vm2"), Some("bob"));
        assert_eq!(lab.ssh_usernames.len(), 1);
    }

    #[test]
    fn lab_status_001_a_lab_is_running_when_all_its_nodes_are_and_stopped_when_all_are() {
        let states = vec![NodeState::Running, NodeState::Running, NodeState::Running];
        assert_eq!(LabState::derive(&states), LabState::Running);
        let states = vec![NodeState::Stopped, NodeState::Stopped];
        assert_eq!(LabState::derive(&states), LabState::Stopped);
    }

    #[test]
    fn lab_status_002_a_lab_is_failed_when_any_node_is_failed_or_missing() {
        let states = vec![NodeState::Running, NodeState::Failed];
        assert_eq!(LabState::derive(&states), LabState::Failed);
        let states = vec![NodeState::Running, NodeState::Missing];
        assert_eq!(LabState::derive(&states), LabState::Failed);
    }

    #[test]
    fn lab_status_003_a_lab_is_starting_when_any_node_is_building_or_starting() {
        let states = vec![NodeState::Running, NodeState::Starting];
        assert_eq!(LabState::derive(&states), LabState::Starting);
        // A lab `up` is still building is not shown stopped.
        let states = vec![NodeState::Building, NodeState::Stopped];
        assert_eq!(LabState::derive(&states), LabState::Starting);
        assert_eq!(
            LabState::derive(&[NodeState::Building, NodeState::Building]),
            LabState::Starting
        );
    }

    #[test]
    fn lab_status_004_a_lab_with_some_nodes_running_and_some_stopped_is_partial() {
        let states = vec![NodeState::Running, NodeState::Stopped];
        assert_eq!(LabState::derive(&states), LabState::Partial);
    }

    #[test]
    fn lab_status_007_a_lab_status_this_version_does_not_know_reads_as_unknown() {
        // A status added by a newer server must not fail an older client's listing.
        let status: LabState = serde_json::from_str(r#""degraded""#).unwrap();
        assert_eq!(status, LabState::Unknown);
    }

    #[test]
    fn lab_status_008_a_lab_with_no_nodes_is_empty() {
        assert_eq!(LabState::derive(&[]), LabState::Empty);
    }

    #[test]
    fn a_summary_from_a_server_without_phases_is_ready() {
        // Servers before 0.5.27 send no phase.
        let summary: LabSummary = serde_json::from_str(
            r#"{"id":"ab12cd34","name":"core","node_count":1,"status":"running"}"#,
        )
        .unwrap();
        assert_eq!(summary.phase, LabPhase::Ready);
    }
}
