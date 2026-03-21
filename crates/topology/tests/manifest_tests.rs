use std::net::{Ipv4Addr, Ipv6Addr};

use shared::data::NodeModel;
use topology::{Bridge, Link2, Manifest, Node, VolumeMount};

// ============================================================================
// Expected TOML manifests
// ============================================================================

const MINIMAL_MANIFEST: &str = r#"
name = "test-lab"

nodes = [
  { name = "dev01", model = "ubuntu_linux" },
]
"#;

const FULL_MANIFEST: &str = r#"
name = "full-lab"
ready_timeout = 600

[[nodes]]
name = "router1"
model = "cisco_iosv"
version = "15.9"
cpu_count = 2
memory = 2048
boot_disk_size = 16
ipv4_address = "10.0.0.1"
ipv6_address = "fd00::1"
skip_ready_check = true
ztp_config = "configs/router1.cfg"
commands = ["show version", "show ip route"]
environment_variables = ["TERM=xterm", "LANG=en_US.UTF-8"]
startup_scripts = ["scripts/init.sh"]

[[nodes]]
name = "server1"
model = "ubuntu_linux"
privileged = true
shm_size = 67108864
user = "root"
volumes = [
  { src = "/data", dst = "/mnt/data" },
  { src = "/logs", dst = "/var/log/app" },
]
ssh_authorized_keys = ["ssh-rsa AAAA... user@host"]

[[links]]
src = "router1::eth1"
dst = "server1::eth1"

[[links]]
src = "router1::eth2"
dst = "server1::eth2"
p2p = true

[[bridges]]
name = "mgmt-bridge"
links = ["router1::eth3", "server1::eth3"]

[ztp_server]
enable = true
username = "admin"
password = "secret123"

[config_management]
ansible = true
pyats = true
nornir = false
"#;

const MANIFEST_WITH_VOLUMES: &str = r#"
name = "vol-lab"

[[nodes]]
name = "app1"
model = "ubuntu_linux"
volumes = [
  { src = "/host/data", dst = "/container/data" },
  { src = "/host/config", dst = "/etc/app" },
]
"#;

// ============================================================================
// Tests — manifest parsing
// ============================================================================

#[test]
fn test_parse_minimal_manifest() {
    let manifest: Manifest = toml::from_str(MINIMAL_MANIFEST).expect("parses");
    assert_eq!(manifest.name, "test-lab");
    assert_eq!(manifest.nodes.len(), 1);
    assert_eq!(manifest.nodes[0].name, "dev01");
    assert_eq!(manifest.nodes[0].model, NodeModel::UbuntuLinux);
    assert!(manifest.links.is_none());
    assert!(manifest.bridges.is_none());
    assert!(manifest.ready_timeout.is_none());
    assert!(manifest.ztp_server.is_none());
    assert!(manifest.config_management.is_none());
}

#[test]
fn test_parse_full_manifest() {
    let manifest: Manifest = toml::from_str(FULL_MANIFEST).expect("parses");

    // Top-level fields
    assert_eq!(manifest.name, "full-lab");
    assert_eq!(manifest.ready_timeout, Some(600));

    // Node 1 — router with all optional fields
    let r1 = &manifest.nodes[0];
    assert_eq!(r1.name, "router1");
    assert_eq!(r1.model, NodeModel::CiscoIosv);
    assert_eq!(r1.version, Some("15.9".to_string()));
    assert_eq!(r1.cpu_count, Some(2));
    assert_eq!(r1.memory, Some(2048));
    assert_eq!(r1.boot_disk_size, Some(16));
    assert_eq!(r1.ipv4_address, Some(Ipv4Addr::new(10, 0, 0, 1)));
    assert_eq!(
        r1.ipv6_address,
        Some("fd00::1".parse::<Ipv6Addr>().unwrap())
    );
    assert_eq!(r1.skip_ready_check, Some(true));
    assert_eq!(r1.ztp_config, Some("configs/router1.cfg".to_string()));
    assert_eq!(
        r1.commands,
        Some(vec![
            "show version".to_string(),
            "show ip route".to_string()
        ])
    );
    assert_eq!(
        r1.environment_variables,
        Some(vec![
            "TERM=xterm".to_string(),
            "LANG=en_US.UTF-8".to_string()
        ])
    );
    assert_eq!(
        r1.startup_scripts,
        Some(vec!["scripts/init.sh".to_string()])
    );

    // Node 2 — container-style with volumes, privileged, user
    let s1 = &manifest.nodes[1];
    assert_eq!(s1.name, "server1");
    assert_eq!(s1.model, NodeModel::UbuntuLinux);
    assert_eq!(s1.privileged, Some(true));
    assert_eq!(s1.shm_size, Some(67108864));
    assert_eq!(s1.user, Some("root".to_string()));
    let vols = s1.volumes.as_ref().expect("has volumes");
    assert_eq!(vols.len(), 2);
    assert_eq!(vols[0].src, "/data");
    assert_eq!(vols[0].dst, "/mnt/data");
    assert_eq!(vols[1].src, "/logs");
    assert_eq!(vols[1].dst, "/var/log/app");
    let keys = s1.ssh_authorized_keys.as_ref().expect("has keys");
    assert_eq!(keys[0], "ssh-rsa AAAA... user@host");

    // Links
    let links = manifest.links.as_ref().expect("has links");
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].src, "router1::eth1");
    assert_eq!(links[0].dst, "server1::eth1");
    assert_eq!(links[0].p2p, None);
    assert_eq!(links[1].src, "router1::eth2");
    assert_eq!(links[1].dst, "server1::eth2");
    assert_eq!(links[1].p2p, Some(true));

    // Bridges
    let bridges = manifest.bridges.as_ref().expect("has bridges");
    assert_eq!(bridges.len(), 1);
    assert_eq!(bridges[0].name, "mgmt-bridge");
    assert_eq!(bridges[0].links, vec!["router1::eth3", "server1::eth3"]);

    // ZTP server
    let ztp = manifest.ztp_server.as_ref().expect("has ztp_server");
    assert_eq!(ztp.enable, true);
    assert_eq!(ztp.username, Some("admin".to_string()));
    assert_eq!(ztp.password, Some("secret123".to_string()));

    // Config management
    let cm = manifest
        .config_management
        .as_ref()
        .expect("has config_management");
    assert_eq!(cm.ansible, true);
    assert_eq!(cm.pyats, true);
    assert_eq!(cm.nornir, false);
}

#[test]
fn test_parse_volumes() {
    let manifest: Manifest = toml::from_str(MANIFEST_WITH_VOLUMES).expect("parses");
    let vols = manifest.nodes[0].volumes.as_ref().expect("has volumes");
    assert_eq!(vols.len(), 2);
    assert_eq!(vols[0].src, "/host/data");
    assert_eq!(vols[0].dst, "/container/data");
    assert_eq!(vols[1].src, "/host/config");
    assert_eq!(vols[1].dst, "/etc/app");
}

// ============================================================================
// Tests — invalid TOML
// ============================================================================

#[test]
fn test_parse_invalid_toml() {
    let result = toml::from_str::<Manifest>("this is not valid toml {{{}}}");
    assert!(result.is_err());
}

#[test]
fn test_parse_missing_name() {
    let toml_str = r#"
nodes = [
  { name = "dev01", model = "ubuntu_linux" },
]
"#;
    let result = toml::from_str::<Manifest>(toml_str);
    assert!(result.is_err());
}

#[test]
fn test_parse_missing_nodes() {
    let toml_str = r#"
name = "test-lab"
"#;
    let result = toml::from_str::<Manifest>(toml_str);
    assert!(result.is_err());
}

// ============================================================================
// Tests — Manifest::example()
// ============================================================================

#[test]
fn test_example_manifest() {
    let manifest = Manifest::example().expect("generates example");
    assert!(!manifest.name.is_empty());
    assert_eq!(manifest.nodes.len(), 2);
    assert_eq!(manifest.nodes[0].name, "dev01");
    assert_eq!(manifest.nodes[0].model, NodeModel::UbuntuLinux);
    assert_eq!(manifest.nodes[1].name, "dev02");
    assert_eq!(manifest.nodes[1].model, NodeModel::FedoraLinux);

    let links = manifest.links.as_ref().expect("has links");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].src, "dev01::eth1");
    assert_eq!(links[0].dst, "dev02::eth1");
}

// ============================================================================
// Tests — write/load round-trip
// ============================================================================

#[test]
fn test_write_load_roundtrip() {
    let manifest = Manifest {
        name: "roundtrip-lab".to_string(),
        ready_timeout: None,
        nodes: vec![
            Node {
                name: "r1".to_string(),
                model: NodeModel::CiscoIosv,
                ..Default::default()
            },
            Node {
                name: "r2".to_string(),
                model: NodeModel::AristaVeos,
                ..Default::default()
            },
        ],
        links: Some(vec![Link2 {
            src: "r1::eth1".to_string(),
            dst: "r2::eth1".to_string(),
            p2p: None,
        }]),
        bridges: None,
        ztp_server: None,
        config_management: None,
    };

    let tmp_path = "/tmp/sherpa_test_manifest_roundtrip.toml";
    manifest.write_file(tmp_path).expect("writes file");
    let loaded = Manifest::load_file(tmp_path).expect("loads file");

    assert_eq!(loaded.name, "roundtrip-lab");
    assert_eq!(loaded.nodes.len(), 2);
    assert_eq!(loaded.nodes[0].name, "r1");
    assert_eq!(loaded.nodes[0].model, NodeModel::CiscoIosv);
    assert_eq!(loaded.nodes[1].name, "r2");
    assert_eq!(loaded.nodes[1].model, NodeModel::AristaVeos);

    let links = loaded.links.as_ref().expect("has links");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].src, "r1::eth1");
    assert_eq!(links[0].dst, "r2::eth1");

    std::fs::remove_file(tmp_path).ok();
}

// ============================================================================
// Tests — Link2::expand()
// ============================================================================

#[test]
fn test_link2_expand() {
    let link = Link2 {
        src: "router1::eth1".to_string(),
        dst: "switch1::GigabitEthernet0/1".to_string(),
        p2p: None,
    };
    let expanded = link.expand().expect("expands");
    assert_eq!(expanded.node_a, "router1");
    assert_eq!(expanded.int_a, "eth1");
    assert_eq!(expanded.node_b, "switch1");
    assert_eq!(expanded.int_b, "GigabitEthernet0/1");
}

#[test]
fn test_link2_expand_missing_separator() {
    let link = Link2 {
        src: "router1-eth1".to_string(),
        dst: "switch1::eth1".to_string(),
        p2p: None,
    };
    let result = link.expand();
    assert!(result.is_err());
}

// ============================================================================
// Tests — Bridge::parse_links()
// ============================================================================

#[test]
fn test_bridge_parse_links() {
    let bridge = Bridge {
        name: "br0".to_string(),
        links: vec![
            "router1::eth3".to_string(),
            "switch1::eth5".to_string(),
            "server1::eth2".to_string(),
        ],
    };
    let expanded = bridge.parse_links().expect("parses");
    assert_eq!(expanded.name, "br0");
    assert_eq!(expanded.links.len(), 3);
    assert_eq!(expanded.links[0].node, "router1");
    assert_eq!(expanded.links[0].interface, "eth3");
    assert_eq!(expanded.links[1].node, "switch1");
    assert_eq!(expanded.links[1].interface, "eth5");
    assert_eq!(expanded.links[2].node, "server1");
    assert_eq!(expanded.links[2].interface, "eth2");
}

#[test]
fn test_bridge_parse_links_invalid_format() {
    let bridge = Bridge {
        name: "br0".to_string(),
        links: vec![
            "router1::eth3".to_string(),
            "bad-format-no-separator".to_string(),
        ],
    };
    let result = bridge.parse_links();
    assert!(result.is_err());
}

// ============================================================================
// Tests — Node defaults
// ============================================================================

#[test]
fn test_node_defaults() {
    let node = Node {
        name: "test".to_string(),
        model: NodeModel::UbuntuLinux,
        ..Default::default()
    };
    assert!(node.version.is_none());
    assert!(node.cpu_count.is_none());
    assert!(node.memory.is_none());
    assert!(node.boot_disk_size.is_none());
    assert!(node.ipv4_address.is_none());
    assert!(node.ipv6_address.is_none());
    assert!(node.volumes.is_none());
    assert!(node.commands.is_none());
    assert!(node.environment_variables.is_none());
    assert!(node.privileged.is_none());
    assert!(node.skip_ready_check.is_none());
    assert!(node.ztp_config.is_none());
    assert!(node.startup_scripts.is_none());
}
