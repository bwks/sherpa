use std::net::Ipv4Addr;

use shared::util::base64_decode;
use template::{
    FileMode, IgnitionConfig, IgnitionFile, IgnitionFileContents, IgnitionLink, IgnitionUnit,
    IgnitionUser,
};

use crate::helpers;

/// The Docker Compose sysext version the fixtures name.
const DOCKER_COMPOSE_VERSION: &str = "2.34.0";

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_FULL_CONFIG: &str = include_str!("fixtures/full_config.json");

const EXPECTED_ZTP_IPV4: &str = "[Match]
Name=eth0

[Network]
Address=172.20.0.10/24
Gateway=172.20.0.1
DNS=172.20.0.1
Domains=sherpa.lab.local
";

const EXPECTED_ZTP_DUAL_STACK: &str = "[Match]
Name=eth0

[Network]
Address=172.20.0.10/24
Gateway=172.20.0.1
DNS=172.20.0.1
Domains=sherpa.lab.local
Address=fd00::10/64
Gateway=fd00::1
DNS=fd00::1
";

const EXPECTED_MOUNT_UNIT: &str = "[Unit]
Before=local-fs.target

[Mount]
What=/dev/disk/by-label/data-disk
Where=/media/container
Type=ext4

[Install]
WantedBy=local-fs.target
";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_ignition_full_config() {
    let user = IgnitionUser {
        name: "core".to_string(),
        password_hash: Some("$6$abc$xyz".to_string()),
        home_dir: None,
        shell: None,
        ssh_authorized_keys: vec!["ssh-rsa AAAA...".to_string()],
        groups: vec!["sudo".to_string(), "docker".to_string()],
    };
    let file = IgnitionFile {
        path: "/etc/systemd/resolved.conf.d/no-stub.conf".to_owned(),
        mode: FileMode::new(0o644),
        overwrite: Some(true),
        contents: Some(IgnitionFileContents::new(
            "data:text/plain;base64,RE5TU3R1Ykxpc3RlbmVyPW5vCg==",
        )),
        ..Default::default()
    };
    let link = IgnitionLink {
        path: "/etc/extensions/docker-compose.raw".to_owned(),
        target: format!(
            "/opt/extensions/docker-compose/docker-compose-{DOCKER_COMPOSE_VERSION}-x86-64.raw"
        ),
        hard: false,
        overwrite: true,
    };
    let unit = IgnitionUnit {
        name: "systemd-resolved.service".to_owned(),
        enabled: Some(false),
        mask: Some(true),
        ..Default::default()
    };

    let config = IgnitionConfig::new(
        vec![user],
        vec![file],
        vec![link],
        vec![unit],
        vec![],
        vec![],
    );
    let json = config.to_json_pretty().expect("renders json");
    assert_eq!(json, EXPECTED_FULL_CONFIG);
}

#[test]
fn test_ignition_ztp_interface_ipv4() {
    let file = IgnitionFile::ztp_interface(
        Ipv4Addr::new(172, 20, 0, 10),
        &helpers::test_network_v4(),
        None,
        None,
    );

    assert_eq!(file.path, "/etc/systemd/network/00-eth0.network");
    assert_eq!(file.mode, FileMode::new(0o644));
    assert_eq!(file.overwrite, Some(true));
    assert_eq!(file.user.as_ref().expect("has user").name, "root");
    assert_eq!(file.group.as_ref().expect("has group").name, "root");

    let b64 = file
        .contents
        .as_ref()
        .expect("has contents")
        .source
        .strip_prefix("data:;base64,")
        .expect("has data uri prefix");
    let decoded = base64_decode(b64).expect("valid base64");
    assert_eq!(decoded, EXPECTED_ZTP_IPV4);
}

#[test]
fn test_ignition_ztp_interface_dual_stack() {
    let v6 = helpers::test_network_v6();
    let file = IgnitionFile::ztp_interface(
        Ipv4Addr::new(172, 20, 0, 10),
        &helpers::test_network_v4(),
        Some("fd00::10".parse().expect("valid")),
        Some(&v6),
    );

    let b64 = file
        .contents
        .as_ref()
        .expect("has contents")
        .source
        .strip_prefix("data:;base64,")
        .expect("has data uri prefix");
    let decoded = base64_decode(b64).expect("valid base64");
    assert_eq!(decoded, EXPECTED_ZTP_DUAL_STACK);
}

#[test]
fn test_ignition_mount_container_disk_unit() {
    let unit = IgnitionUnit::mount_container_disk();
    assert_eq!(unit.name, "media-container.mount");
    assert_eq!(unit.enabled, Some(true));
    assert_eq!(unit.contents.expect("has contents"), EXPECTED_MOUNT_UNIT);
}
