use std::net::Ipv4Addr;

use serde_json::{Value, json};
use template::{
    CloudInitConfig, CloudInitNetwork, CloudInitResolvConf, CloudInitUser, MetaDataConfig,
};

use crate::helpers;

// Guest YAML is written as JSON, a YAML subset in which every string is quoted, so
// YAML 1.1 parsers such as cloud-init's cannot read strings as numbers or booleans.

fn parse_json(document: &str) -> Value {
    serde_json::from_str(document).expect("guest config is JSON")
}

fn parse_cloud_config(document: &str) -> Value {
    let body = document
        .strip_prefix("#cloud-config\n")
        .expect("user-data starts with the #cloud-config line");
    parse_json(body)
}

fn network(mac_address: &str) -> CloudInitNetwork {
    CloudInitNetwork::ztp_interface(
        Ipv4Addr::new(172, 20, 0, 10),
        mac_address.to_string(),
        helpers::test_network_v4(),
        None,
        None,
    )
}

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
    let output = config.to_string().expect("renders user-data");
    assert_eq!(
        parse_cloud_config(&output),
        json!({
            "hostname": "ubuntu01",
            "fqdn": "ubuntu01.lab.sherpa.local",
            "manage_etc_hosts": true,
            "ssh_pwauth": true,
            "users": [],
            "packages": ["curl", "vim"],
            "runcmd": ["echo hello"],
        })
    );
}

#[test]
fn test_cloud_init_resolv_conf_points_at_the_lab_dns() {
    let config = CloudInitConfig {
        manage_resolv_conf: Some(true),
        resolv_conf: Some(CloudInitResolvConf::lab_dns(Ipv4Addr::new(172, 20, 0, 2))),
        ..Default::default()
    };
    let output = parse_cloud_config(&config.to_string().expect("renders user-data"));
    assert_eq!(output["manage_resolv_conf"], json!(true));
    assert_eq!(
        output["resolv_conf"],
        json!({
            "domain": "sherpa.lab.local",
            "searchdomains": ["sherpa.lab.local"],
            "nameservers": ["172.20.0.2"],
        })
    );
}

#[test]
fn test_cloud_init_user_without_groups_omits_the_key() {
    let config = CloudInitConfig {
        users: vec![CloudInitUser {
            name: "alice".to_string(),
            plain_text_passwd: None,
            hashed_passwd: None,
            homedir: None,
            lock_passwd: true,
            ssh_authorized_keys: vec![],
            sudo: "ALL=(ALL) NOPASSWD:ALL".to_string(),
            groups: vec![],
            shell: "/bin/bash".to_string(),
        }],
        ..Default::default()
    };
    let output = parse_cloud_config(&config.to_string().expect("renders user-data"));
    assert_eq!(output["users"][0].get("groups"), None);
}

#[test]
fn test_cloud_init_metadata() {
    let meta = MetaDataConfig {
        instance_id: "i-ubuntu01".to_string(),
        local_hostname: "ubuntu01".to_string(),
        public_keys: vec!["ssh-rsa AAAA... test@host".to_string()],
    };
    let output = meta.to_string().expect("renders meta-data");
    assert_eq!(
        parse_json(&output),
        json!({
            "instance-id": "i-ubuntu01",
            "local-hostname": "ubuntu01",
            "public-keys": ["ssh-rsa AAAA... test@host"],
        })
    );
}

#[test]
fn test_cloud_init_network_ipv4() {
    let output = network("52:54:00:aa:bb:cc")
        .to_string()
        .expect("renders network-config");
    assert_eq!(
        parse_json(&output),
        json!({
            "version": 2,
            "ethernets": {
                "id0": {
                    "match": { "macaddress": "52:54:00:aa:bb:cc" },
                    "addresses": ["172.20.0.10/24"],
                    "routes": [{ "to": "0.0.0.0/0", "via": "172.20.0.1" }],
                    "nameservers": {
                        "addresses": ["172.20.0.1"],
                        "search": ["sherpa.lab.local"],
                    },
                },
            },
        })
    );
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
    let output = net.to_string().expect("renders network-config");
    assert_eq!(
        parse_json(&output)["ethernets"]["id0"],
        json!({
            "match": { "macaddress": "52:54:00:aa:bb:cc" },
            "addresses": ["172.20.0.10/24", "fd00::10/64"],
            "routes": [
                { "to": "0.0.0.0/0", "via": "172.20.0.1" },
                { "to": "::/0", "via": "fd00::1" },
            ],
            "nameservers": {
                "addresses": ["172.20.0.1", "fd00::1"],
                "search": ["sherpa.lab.local"],
            },
        })
    );
}

/// An all-digit MAC is a base-60 integer in YAML 1.1 when unquoted.
#[test]
fn test_all_digit_mac_address_is_a_quoted_string() {
    let output = network("52:54:00:21:47:08")
        .to_string()
        .expect("renders network-config");
    assert!(
        output.contains(r#""macaddress": "52:54:00:21:47:08""#),
        "MAC address must be a quoted string: {output}"
    );
    assert_eq!(
        parse_json(&output)["ethernets"]["id0"]["match"]["macaddress"],
        "52:54:00:21:47:08"
    );
}

/// `yes`, `no`, `on` and `off` are booleans in YAML 1.1 when unquoted.
#[test]
fn test_yaml_boolean_words_stay_strings() {
    let config = CloudInitConfig {
        hostname: "on".to_string(),
        fqdn: "no".to_string(),
        users: vec![CloudInitUser {
            name: "off".to_string(),
            plain_text_passwd: Some("yes".to_string()),
            hashed_passwd: None,
            homedir: None,
            lock_passwd: false,
            ssh_authorized_keys: vec![],
            sudo: "ALL=(ALL) NOPASSWD:ALL".to_string(),
            groups: vec![],
            shell: "/bin/bash".to_string(),
        }],
        ..Default::default()
    };
    let parsed = parse_cloud_config(&config.to_string().expect("renders user-data"));
    assert_eq!(parsed["hostname"], "on");
    assert_eq!(parsed["fqdn"], "no");
    assert_eq!(parsed["users"][0]["name"], "off");
    assert_eq!(parsed["users"][0]["plain_text_passwd"], "yes");
}
