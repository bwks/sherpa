use super::TailnetStatus;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::data::{
    BridgeKind, LabInfo, LabPhase, LabState, LinkState, NodeKind, NodeModel, NodeState,
};

/// Request type for inspecting a lab
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectRequest {
    pub lab_id: String,
}

/// Response structure for inspect operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale: Option<TailnetStatus>,
    pub lab_info: LabInfo,
    pub lab_state: LabState,
    /// Which operation owns the lab, if any. Servers before 0.5.27 send none: their labs are
    /// `ready`.
    #[serde(default)]
    pub lab_phase: LabPhase,
    pub devices: Vec<DeviceInfo>,
    pub inactive_devices: Vec<String>,
    pub links: Vec<LinkInfo>,
    pub bridges: Vec<BridgeInfo>,
}

/// Information about a single device/node
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeviceInfo {
    pub name: String,
    pub model: NodeModel,
    pub kind: NodeKind,
    pub state: NodeState,
    pub mgmt_ipv4: String,
    pub mgmt_ipv6: Option<String>,
    pub disks: Vec<String>,
    /// vCPUs the node was given: a container's CPU quota. `None` for a container without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_count: Option<u8>,
    /// Memory in MB the node was given; `None` as for `cpu_count`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<u16>,
}

/// Display-ready information about a point-to-point link between two nodes
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinkInfo {
    pub node_a_name: String,
    pub int_a: String,
    pub node_b_name: String,
    pub int_b: String,
    pub kind: BridgeKind,
    /// What the server last found or left the link in: a scan of the lab, a resume or a
    /// redeploy. `None` when the server has not checked it since it started, and from servers
    /// that do not report link states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<LinkState>,
    /// The link's number in the lab, as `link impair` names it to the server. `None` from
    /// servers that do not report it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u16>,
}

/// Display-ready information about a shared bridge connecting multiple nodes
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BridgeInfo {
    pub bridge_name: String,
    pub network_name: String,
    pub connected_nodes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(state: Option<LinkState>) -> LinkInfo {
        LinkInfo {
            node_a_name: "vm1".to_string(),
            int_a: "eth1".to_string(),
            node_b_name: "ctr1".to_string(),
            int_b: "eth1".to_string(),
            kind: BridgeKind::P2p,
            state,
            index: None,
        }
    }

    #[test]
    fn link_state_007_a_link_from_a_server_with_no_state_or_index_still_reads() {
        let checked = serde_json::to_value(link(Some(LinkState::PeerDown {
            peer: "ctr1".to_string(),
        })))
        .unwrap();
        assert_eq!(
            checked["state"],
            serde_json::json!({"state": "peer_down", "peer": "ctr1"})
        );
        let unchecked = serde_json::to_value(link(None)).unwrap();
        assert!(unchecked.get("state").is_none(), "{unchecked}");
        // A server before link states sends none.
        let mut old = unchecked;
        let back: LinkInfo = serde_json::from_value(old.take()).unwrap();
        assert_eq!(back.state, None);

        let mut indexed = link(None);
        indexed.index = Some(3);
        let value = serde_json::to_value(&indexed).unwrap();
        assert_eq!(value["index"], serde_json::json!(3));
        let back: LinkInfo = serde_json::from_value(value).unwrap();
        assert_eq!(back.index, Some(3));

        let mut old = serde_json::to_value(link(None)).unwrap();
        assert!(old.get("index").is_none(), "{old}");
        let back: LinkInfo = serde_json::from_value(old.take()).unwrap();
        assert_eq!(back.index, None);
    }

    fn device() -> DeviceInfo {
        DeviceInfo {
            name: "vm1".to_string(),
            model: NodeModel::AlpineLinux,
            kind: NodeKind::VirtualMachine,
            state: NodeState::Running,
            mgmt_ipv4: "172.31.0.11".to_string(),
            mgmt_ipv6: None,
            disks: Vec::new(),
            cpu_count: None,
            memory: None,
        }
    }

    #[test]
    fn a_device_from_an_older_server_with_a_console_address_still_reads() {
        // The console is reached through sherpad's API, not at an address (#674).
        let mut value = serde_json::to_value(device()).unwrap();
        value["console_address"] = serde_json::json!("127.127.3.1");
        let back: DeviceInfo = serde_json::from_value(value).unwrap();
        assert_eq!(back.name, "vm1");
    }
}
