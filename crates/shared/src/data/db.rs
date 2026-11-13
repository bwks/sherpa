use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{BridgeKind, LabPhase, LinkImpairment, NodeState, RecordId};

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

/// Where a session was started: the CLI's RPC login, the REST API, or the web UI.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionChannel {
    Cli,
    Api,
    Web,
}

/// A login session. Every token names one; the token is accepted only while this row exists
/// and `expires_at` has not passed.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct DbSession {
    /// The session's key, carried in the token as `sid`.
    pub id: String,
    pub user: RecordId,
    pub channel: SessionChannel,
    pub created_at: Timestamp,
    /// When the session expires, to the second, the same as the token's `exp`.
    pub expires_at: Timestamp,
}

/// A session with the current name and admin flag of its user, read in one query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbSessionUser {
    pub session: DbSession,
    pub username: String,
    pub is_admin: bool,
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
    /// Which operation owns the lab, if any. Its status is derived from its nodes when read.
    #[serde(default)]
    pub phase: LabPhase,
    /// The hostname of the machine `sherpa up` ran on. Labs started from the web UI or the API
    /// have none.
    #[serde(default)]
    pub client_host: Option<String>,
    /// The folder `sherpa up` ran in, on that machine.
    #[serde(default)]
    pub client_folder: Option<String>,
    /// The lab's description, from its manifest.
    #[serde(default)]
    pub description: Option<String>,
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
    /// vCPUs the node was given: a container's CPU quota. `None` for a container without one.
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
    /// Host CPUs the node's vCPUs are pinned to, in vCPU order. `None` when unpinned.
    #[serde(default)]
    pub cpu_pinning: Option<Vec<u16>>,
    /// Host CPUs the node's QEMU and I/O threads are pinned to. `None` when unpinned.
    #[serde(default)]
    pub qemu_thread_cpus: Option<Vec<u16>>,
}

/// What a node was given when it was created or redeployed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeResources {
    pub cpu_count: Option<u8>,
    pub memory: Option<u16>,
    pub vnc_port: Option<u16>,
    pub cpu_pinning: Option<Vec<u16>>,
    pub qemu_thread_cpus: Option<Vec<u16>>,
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
    /// Tap device name for `node_a` side (P2p links only).
    #[serde(default)]
    pub tap_a: String,
    /// Tap device name for `node_b` side (P2p links only).
    #[serde(default)]
    pub tap_b: String,
    /// Impairment on traffic from `node_a` to `node_b`: netem on `tap_b`'s egress.
    #[serde(default)]
    pub impairment_a_to_b: LinkImpairment,
    /// Impairment on traffic from `node_b` to `node_a`: netem on `tap_a`'s egress.
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
