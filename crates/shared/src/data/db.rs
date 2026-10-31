use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{BridgeKind, LabState, LinkImpairment, NodeState, RecordId};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbUser {
    pub id: Option<RecordId>,
    pub username: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub ssh_keys: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbLab {
    pub id: Option<RecordId>,
    pub lab_id: String,
    pub name: String,
    pub user: RecordId,
    pub loopback_network: String,
    pub management_network: String,
    pub gateway_ipv4: String,
    pub router_ipv4: String,
    pub tailscale_container_id: Option<String>,
    pub management_network_v6: Option<String>,
    pub gateway_ipv6: Option<String>,
    pub router_ipv6: Option<String>,
    pub loopback_network_v6: Option<String>,
    #[serde(default)]
    pub status: LabState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbNode {
    pub id: Option<RecordId>,
    pub name: String,
    pub image: RecordId,
    pub index: u16,
    pub lab: RecordId,
    pub mgmt_ipv4: Option<String>,
    pub mgmt_ipv6: Option<String>,
    pub mgmt_mac: Option<String>,
    pub state: NodeState,
    /// vCPUs the node was given. `None` for containers, which Sherpa does not limit.
    #[serde(default)]
    pub cpu_count: Option<u8>,
    /// Memory in MB the node was given; `None` as for `cpu_count`.
    #[serde(default)]
    pub memory: Option<u16>,
    /// The VM's VNC console port on the server, fixed for the node's life. `None` for nodes
    /// without a console.
    #[serde(default)]
    pub vnc_port: Option<u16>,
    /// The SSH host key the server recorded when the node last became ready, in OpenSSH
    /// public key form (`ssh-ed25519 AAAA...`). `None` for nodes without SSH, or before then.
    #[serde(default)]
    pub ssh_host_key: Option<String>,
}

/// What a node was given when it was created or redeployed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeResources {
    pub cpu_count: Option<u8>,
    pub memory: Option<u16>,
    pub vnc_port: Option<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbLink {
    pub id: Option<RecordId>,
    pub index: u16,
    pub kind: BridgeKind,
    pub node_a: RecordId,
    pub node_b: RecordId,
    pub int_a: String,
    pub int_b: String,
    pub lab: RecordId,
    /// Tap device name for node_a side (P2p links only).
    #[serde(default)]
    pub tap_a: String,
    /// Tap device name for node_b side (P2p links only).
    #[serde(default)]
    pub tap_b: String,
    /// Impairment on traffic from node_a to node_b: netem on `tap_b`'s egress.
    #[serde(default)]
    pub impairment_a_to_b: LinkImpairment,
    /// Impairment on traffic from node_b to node_a: netem on `tap_a`'s egress.
    #[serde(default)]
    pub impairment_b_to_a: LinkImpairment,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbBridge {
    pub id: Option<RecordId>,
    pub index: u16,
    pub bridge_name: String,
    pub network_name: String,
    pub lab: RecordId,
    pub members: Vec<BridgeMember>,
}

/// A node interface connected to a shared bridge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BridgeMember {
    pub node: RecordId,
    pub interface: String,
}
