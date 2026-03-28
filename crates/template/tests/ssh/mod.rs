use std::net::Ipv4Addr;

use askama::Template;

use shared::data::{ZtpMethod, ZtpRecord};
use template::SshConfigTemplate;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_SSH_CONFIG: &str = "
Host 172.20.0.10 router1.abcd1234 router1.abcd1234.sherpa.lab.local
    HostName 172.20.0.10
    Port 22
    User sherpa
    ProxyJump sherpa@10.0.0.1
    IdentityFile sherpa_ssh_key
    IdentitiesOnly yes
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null
    PubkeyAcceptedAlgorithms +ssh-rsa
    HostkeyAlgorithms +ssh-rsa
    KexAlgorithms +diffie-hellman-group-exchange-sha1,diffie-hellman-group14-sha1";

const EXPECTED_SSH_CONFIG_MULTI: &str = "
Host 172.20.0.10 router1.abcd1234 router1.abcd1234.sherpa.lab.local
    HostName 172.20.0.10
    Port 22
    User sherpa
    ProxyJump sherpa@10.0.0.1
    IdentityFile sherpa_ssh_key
    IdentitiesOnly yes
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null
    PubkeyAcceptedAlgorithms +ssh-rsa
    HostkeyAlgorithms +ssh-rsa
    KexAlgorithms +diffie-hellman-group-exchange-sha1,diffie-hellman-group14-sha1
Host 172.20.0.11 switch1.abcd1234 switch1.abcd1234.sherpa.lab.local
    HostName 172.20.0.11
    Port 830
    User sherpa
    ProxyJump sherpa@10.0.0.1
    IdentityFile sherpa_ssh_key
    IdentitiesOnly yes
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null
    PubkeyAcceptedAlgorithms +ssh-rsa
    HostkeyAlgorithms +ssh-rsa
    KexAlgorithms +diffie-hellman-group-exchange-sha1,diffie-hellman-group14-sha1";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_ssh_config() {
    let t = SshConfigTemplate {
        ztp_records: vec![ZtpRecord {
            node_name: "router1".to_string(),
            config_file: "router1.cfg".to_string(),
            ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
            ipv6_address: None,
            mac_address: "52:54:00:aa:bb:cc".to_string(),
            ztp_method: ZtpMethod::None,
            ssh_port: 22,
        }],
        proxy_user: "sherpa".to_string(),
        server_ipv4: "10.0.0.1".to_string(),
        lab_id: "abcd1234".to_string(),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_SSH_CONFIG);
}

#[test]
fn test_ssh_config_multiple_hosts() {
    let t = SshConfigTemplate {
        ztp_records: vec![
            ZtpRecord {
                node_name: "router1".to_string(),
                config_file: "router1.cfg".to_string(),
                ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
                ipv6_address: None,
                mac_address: "52:54:00:aa:bb:cc".to_string(),
                ztp_method: ZtpMethod::None,
                ssh_port: 22,
            },
            ZtpRecord {
                node_name: "switch1".to_string(),
                config_file: "switch1.cfg".to_string(),
                ipv4_address: Ipv4Addr::new(172, 20, 0, 11),
                ipv6_address: None,
                mac_address: "52:54:00:dd:ee:ff".to_string(),
                ztp_method: ZtpMethod::None,
                ssh_port: 830,
            },
        ],
        proxy_user: "sherpa".to_string(),
        server_ipv4: "10.0.0.1".to_string(),
        lab_id: "abcd1234".to_string(),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_SSH_CONFIG_MULTI);
}
