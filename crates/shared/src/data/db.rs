use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{BridgeKind, LabPhase, LinkImpairment, NodeCpuPins, NodeState, RecordId};

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
    pub id: String,
    pub user: RecordId,
    pub channel: SessionChannel,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

/// One login attempt on an account that was not locked: when it was made, whether it
/// succeeded, and the address it came from.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct DbLoginAttempt {
    pub id: String,
    pub user: RecordId,
    pub attempted_at: Timestamp,
    pub succeeded: bool,
    pub address: IpAddr,
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
    pub management_network: String,
    pub gateway_ipv4: String,
    pub router_ipv4: String,
    pub tailscale_container_id: Option<String>,
    pub management_network_v6: Option<String>,
    pub gateway_ipv6: Option<String>,
    pub router_ipv6: Option<String>,
    pub loopback_network_v6: Option<String>,
    #[serde(default)]
    pub phase: LabPhase,
    #[serde(default)]
    pub client_host: Option<String>,
    #[serde(default)]
    pub client_folder: Option<String>,
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
    #[serde(default)]
    pub cpu_count: Option<u8>,
    #[serde(default)]
    pub memory: Option<u16>,
    #[serde(default)]
    pub ssh_host_key: Option<String>,
    #[serde(default)]
    pub ready_check: ReadyCheck,
    #[serde(default)]
    pub ssh_username: Option<String>,
}

/// A node's readiness check, stored with the node so that the scanner, `resume` and `redeploy`
/// decide readiness alike without the manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyCheck {
    pub port: Option<u16>,
    pub skip: bool,
    pub sshd_marker: bool,
}

/// What a node was given when it was created or redeployed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeResources {
    pub cpu_count: Option<u8>,
    pub memory: Option<u16>,
}

/// The host CPUs a node holds, as the `cpu_pin` table stores them: its lab, and its pins under
/// its name in the lab, each list in ascending order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbCpuPins {
    pub lab: RecordId,
    pub pins: NodeCpuPins,
}

/// A sandbox's own record, as the `vm_sandbox` table stores it: its node, its own management
/// addresses, the MAC of its NIC and the SSH host key its sshd offers, once recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbVmSandbox {
    pub node: RecordId,
    pub ipv4: Ipv4Addr,
    pub ipv6: Option<Ipv6Addr>,
    pub mac: String,
    pub ssh_host_key: Option<String>,
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
    #[serde(default)]
    pub tap_a: String,
    #[serde(default)]
    pub tap_b: String,
    #[serde(default)]
    pub impairment_a_to_b: LinkImpairment,
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
