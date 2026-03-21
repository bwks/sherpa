use std::collections::HashMap;
use std::net::Ipv4Addr;

use shared::data::{NodeConfig, NodeModel, ZtpMethod, ZtpRecord};
use template::PyatsInventory;
use topology::Manifest;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_PYATS_INVENTORY: &str = "devices:
  router1:
    alias: router1
    connections:
      mgmt:
        ip: 172.20.0.10
        protocol: ssh
        port: 22
        ssh_options: -F .tmp/sherpa_ssh_config
    credentials:
      default:
        password: admin123
        username: admin
    os: linux
    platform: linux
    type: cisco_iosv
";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_pyats_inventory_from_manifest() {
    let manifest = Manifest {
        name: "test-lab".to_string(),
        ready_timeout: None,
        nodes: vec![topology::Node {
            name: "router1".to_string(),
            model: NodeModel::CiscoIosv,
            ..Default::default()
        }],
        links: None,
        bridges: None,
        ztp_server: None,
        config_management: None,
    };

    let node_config = helpers::test_node_config(NodeModel::CiscoIosv);
    let mut node_images: HashMap<NodeModel, NodeConfig> = HashMap::new();
    node_images.insert(NodeModel::CiscoIosv, node_config);

    let device_ips = vec![ZtpRecord {
        node_name: "router1".to_string(),
        config_file: "router1.cfg".to_string(),
        ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        ipv6_address: None,
        mac_address: "52:54:00:aa:bb:cc".to_string(),
        ztp_method: ZtpMethod::None,
        ssh_port: 22,
    }];

    let inventory = PyatsInventory::from_manifest(
        &manifest,
        &node_images,
        &device_ips,
        Some("admin".to_string()),
        Some("admin123".to_string()),
    )
    .expect("builds inventory");

    let yaml = inventory.to_yaml().expect("serializes to yaml");
    assert_eq!(yaml, EXPECTED_PYATS_INVENTORY);
}
