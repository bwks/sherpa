use std::net::Ipv4Addr;

use template::{CloudInitConfig, CloudInitNetwork, MetaDataConfig};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_CLOUD_INIT_CONFIG: &str = "#cloud-config
hostname: ubuntu01
fqdn: ubuntu01.lab.sherpa.local
manage_etc_hosts: true
ssh_pwauth: true
users: []
packages:
- curl
- vim
runcmd:
- echo hello
";

const EXPECTED_METADATA: &str = "instance-id: i-ubuntu01
local-hostname: ubuntu01
public-keys:
- ssh-rsa AAAA... test@host
";

const EXPECTED_NETWORK_IPV4: &str = "version: 2
ethernets:
  id0:
    match:
      macaddress: 52:54:00:aa:bb:cc
    addresses:
    - 172.20.0.10/24
    routes:
    - to: 0.0.0.0/0
      via: 172.20.0.1
    nameservers:
      addresses:
      - 172.20.0.1
      search:
      - sherpa.lab.local
";

const EXPECTED_NETWORK_DUAL_STACK: &str = "version: 2
ethernets:
  id0:
    match:
      macaddress: 52:54:00:aa:bb:cc
    addresses:
    - 172.20.0.10/24
    - fd00::10/64
    routes:
    - to: 0.0.0.0/0
      via: 172.20.0.1
    - to: ::/0
      via: fd00::1
    nameservers:
      addresses:
      - 172.20.0.1
      - fd00::1
      search:
      - sherpa.lab.local
";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_cloud_init_config() {
    let config = CloudInitConfig {
        hostname: "ubuntu01".to_string(),
        fqdn: "ubuntu01.lab.sherpa.local".to_string(),
        manage_etc_hosts: true,
        ssh_pwauth: true,
        users: vec![],
        manage_resolv_conf: None,
        resolv_conf: None,
        packages: Some(vec!["curl".to_string(), "vim".to_string()]),
        write_files: None,
        runcmd: Some(vec!["echo hello".to_string()]),
    };
    let output = config.to_string().expect("renders yaml");
    assert_eq!(output, EXPECTED_CLOUD_INIT_CONFIG);
}

#[test]
fn test_cloud_init_metadata() {
    let meta = MetaDataConfig {
        instance_id: "i-ubuntu01".to_string(),
        local_hostname: "ubuntu01".to_string(),
        public_keys: vec!["ssh-rsa AAAA... test@host".to_string()],
    };
    let output = meta.to_string().expect("renders yaml");
    assert_eq!(output, EXPECTED_METADATA);
}

#[test]
fn test_cloud_init_network_ipv4() {
    let net = CloudInitNetwork::ztp_interface(
        Ipv4Addr::new(172, 20, 0, 10),
        "52:54:00:aa:bb:cc".to_string(),
        helpers::test_network_v4(),
        None,
        None,
    );
    let output = net.to_string().expect("renders yaml");
    assert_eq!(output, EXPECTED_NETWORK_IPV4);
}

#[test]
fn test_cloud_init_network_dual_stack() {
    let v6 = helpers::test_network_v6();
    let net = CloudInitNetwork::ztp_interface(
        Ipv4Addr::new(172, 20, 0, 10),
        "52:54:00:aa:bb:cc".to_string(),
        helpers::test_network_v4(),
        Some("fd00::10".parse().expect("valid")),
        Some(&v6),
    );
    let output = net.to_string().expect("renders yaml");
    assert_eq!(output, EXPECTED_NETWORK_DUAL_STACK);
}
