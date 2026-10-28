use std::net::Ipv4Addr;

use askama::Template;

use template::{KnownHostsEntry, KnownHostsTemplate, SshConfigTemplate, SshHostConfig};

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
        hosts: vec![SshHostConfig {
            aliases: "172.20.0.10 router1.abcd1234 router1.abcd1234.sherpa.lab.local".to_string(),
            username: "sherpa".to_owned(),
            identity_file: Some("sherpa_ssh_key".to_owned()),
            known_hosts_file: None,
            hostname: Ipv4Addr::new(172, 20, 0, 10),
            port: 22,
        }],
        proxy_jump: "sherpa@10.0.0.1".to_string(),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_SSH_CONFIG);
}

#[test]
fn test_ssh_config_multiple_hosts() {
    let t = SshConfigTemplate {
        hosts: vec![
            SshHostConfig {
                aliases: "172.20.0.10 router1.abcd1234 router1.abcd1234.sherpa.lab.local"
                    .to_string(),
                username: "sherpa".to_owned(),
                identity_file: Some("sherpa_ssh_key".to_owned()),
                known_hosts_file: None,
                hostname: Ipv4Addr::new(172, 20, 0, 10),
                port: 22,
            },
            SshHostConfig {
                aliases: "172.20.0.11 switch1.abcd1234 switch1.abcd1234.sherpa.lab.local"
                    .to_string(),
                username: "sherpa".to_owned(),
                identity_file: Some("sherpa_ssh_key".to_owned()),
                known_hosts_file: None,
                hostname: Ipv4Addr::new(172, 20, 0, 11),
                port: 830,
            },
        ],
        proxy_jump: "sherpa@10.0.0.1".to_string(),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_SSH_CONFIG_MULTI);
}

#[test]
fn a_host_with_a_known_hosts_file_is_checked_strictly() {
    let t = SshConfigTemplate {
        hosts: vec![SshHostConfig {
            aliases: "172.20.0.10 router1.abcd1234 router1.abcd1234.sherpa.lab.local".to_string(),
            username: "sherpa".to_owned(),
            identity_file: Some("sherpa_ssh_key".to_owned()),
            known_hosts_file: Some("sherpa_known_hosts".to_owned()),
            hostname: Ipv4Addr::new(172, 20, 0, 10),
            port: 22,
        }],
        proxy_jump: "sherpa@10.0.0.1".to_string(),
    };
    let output = t.render().expect("template renders");
    let expected = EXPECTED_SSH_CONFIG
        .replace("StrictHostKeyChecking no", "StrictHostKeyChecking yes")
        .replace(
            "UserKnownHostsFile /dev/null",
            "UserKnownHostsFile sherpa_known_hosts",
        );
    assert_eq!(output, expected);
}

#[test]
fn known_hosts_names_port_22_by_address_and_other_ports_in_brackets() {
    let t = KnownHostsTemplate {
        entries: vec![
            KnownHostsEntry {
                hostname: Ipv4Addr::new(172, 20, 0, 10),
                port: 22,
                key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD".to_owned(),
            },
            KnownHostsEntry {
                hostname: Ipv4Addr::new(172, 20, 0, 11),
                port: 830,
                key: "ssh-rsa AAAAB3NzaC1yc2E".to_owned(),
            },
        ],
    };
    let output = t.render().expect("template renders");
    assert_eq!(
        output,
        "172.20.0.10 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD\n\
         [172.20.0.11]:830 ssh-rsa AAAAB3NzaC1yc2E\n"
    );
}

#[test]
fn known_hosts_without_entries_is_empty() {
    let t = KnownHostsTemplate { entries: vec![] };
    assert_eq!(t.render().expect("template renders"), "");
}
