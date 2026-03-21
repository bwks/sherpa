use std::net::Ipv4Addr;

use template::{IgnitionConfig, IgnitionFile, IgnitionLink, IgnitionUnit, IgnitionUser};

use crate::helpers;

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

const EXPECTED_DNSMASQ_UNIT: &str = "[Unit]
Description=dnsmasq
After=media-container.mount containerd.service
Requires=media-container.mount containerd.service

[Service]
TimeoutStartSec=infinity
ExecStartPre=/usr/bin/mkdir -p /opt/ztp/dnsmasq
ExecStartPre=/usr/bin/mkdir -p /opt/ztp/images
ExecStartPre=/usr/bin/touch /opt/ztp/dnsmasq/leases.txt
ExecStartPre=/usr/bin/bash -c 'chmod -R a+r /opt/ztp/'
ExecStartPre=/usr/bin/docker load -i /media/container/dnsmasq.tar.gz
ExecStart=/usr/bin/docker container run --rm --name dnsmasq-app --network host -v /opt/dnsmasq/dnsmasq.conf:/etc/dnsmasq.conf -v /opt/ztp/dnsmasq/leases.txt:/var/lib/misc/dnsmasq.leases -v /opt/ztp/tftp:/opt/ztp/tftp --cap-add=NET_ADMIN dockurr/dnsmasq
ExecStop=/usr/bin/docker container stop dnsmasq-app

Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_ignition_full_config() {
    let user = IgnitionUser {
        name: "core".to_string(),
        password_hash: "$6$abc$xyz".to_string(),
        ssh_authorized_keys: vec!["ssh-rsa AAAA...".to_string()],
        groups: vec!["sudo".to_string(), "docker".to_string()],
    };
    let file = IgnitionFile::disable_resolved();
    let link = IgnitionLink::docker_compose_raw();
    let unit = IgnitionUnit::systemd_resolved();

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
        helpers::test_network_v4(),
        None,
        None,
    )
    .expect("builds file");

    assert_eq!(file.path, "/etc/systemd/network/00-eth0.network");
    assert_eq!(file.mode, 644);
    assert_eq!(file.overwrite, Some(true));
    assert_eq!(file.user.as_ref().expect("has user").name, "root");
    assert_eq!(file.group.as_ref().expect("has group").name, "root");

    let b64 = file
        .contents
        .source
        .strip_prefix("data:;base64,")
        .expect("has data uri prefix");
    let decoded = shared::util::base64_decode(b64).expect("valid base64");
    assert_eq!(decoded, EXPECTED_ZTP_IPV4);
}

#[test]
fn test_ignition_ztp_interface_dual_stack() {
    let v6 = helpers::test_network_v6();
    let file = IgnitionFile::ztp_interface(
        Ipv4Addr::new(172, 20, 0, 10),
        helpers::test_network_v4(),
        Some("fd00::10".parse().expect("valid")),
        Some(&v6),
    )
    .expect("builds file");

    let b64 = file
        .contents
        .source
        .strip_prefix("data:;base64,")
        .expect("has data uri prefix");
    let decoded = shared::util::base64_decode(b64).expect("valid base64");
    assert_eq!(decoded, EXPECTED_ZTP_DUAL_STACK);
}

#[test]
fn test_ignition_mount_container_disk_unit() {
    let unit = IgnitionUnit::mount_container_disk();
    assert_eq!(unit.name, "media-container.mount");
    assert_eq!(unit.enabled, Some(true));
    assert_eq!(unit.contents.expect("has contents"), EXPECTED_MOUNT_UNIT);
}

#[test]
fn test_ignition_dnsmasq_unit() {
    let unit = IgnitionUnit::dnsmasq();
    assert_eq!(unit.name, "dnsmasq.service");
    assert_eq!(unit.enabled, Some(true));
    assert_eq!(unit.contents.expect("has contents"), EXPECTED_DNSMASQ_UNIT);
}
