use askama::Template;
use serde_json::Value;

use shared::util::base64_decode;
use template::{
    GuestAddress, GuestInterface, NODE_VM_CONFIG_DRIVE_MOUNT, NodeVmDomainTemplate, NodeVmGuest,
    NodeVmManagement, NodeVmNic, WritableVolume, store_vm_ignition_json,
};

// ============================================================================
// Helpers
// ============================================================================

fn interface(mac: &str, name: &str) -> GuestInterface {
    GuestInterface {
        mac: mac.to_owned(),
        name: name.to_owned(),
    }
}

fn node_config() -> Value {
    let create_args = [
        "--name".to_owned(),
        "ceos1-abc12345".to_owned(),
        "--env".to_owned(),
        "A=b c".to_owned(),
        "ceos:4.34".to_owned(),
    ];
    let json = NodeVmGuest {
        hostname: "ceos1",
        container: "ceos1-abc12345",
        interfaces: &[
            interface("52:54:00:AB:00:01", "eth0"),
            interface("52:54:00:AB:00:02", "eth1"),
        ],
        placeholders: &[
            interface("52:54:00:AB:00:03", "eth2"),
            interface("52:54:00:AB:00:04", "eth3"),
        ],
        management: &[
            GuestAddress {
                address: "172.31.0.23".parse().unwrap(),
                prefix_length: 24,
                gateway: "172.31.0.1".parse().unwrap(),
            },
            GuestAddress {
                address: "fd00:b0d1:0:1::17".parse().unwrap(),
                prefix_length: 64,
                gateway: "fd00:b0d1:0:1::1".parse().unwrap(),
            },
        ],
        console_command: &["Cli"],
        create_args: &create_args,
        writable: &[WritableVolume {
            source: "/media/config/sherpa/volumes/1".to_owned(),
            destination: "/etc/opt/srlinux/config.json".to_owned(),
        }],
    }
    .ignition_json()
    .unwrap();
    serde_json::from_str(&json).unwrap()
}

fn store_config() -> Value {
    serde_json::from_str(&store_vm_ignition_json().unwrap()).unwrap()
}

fn file<'a>(config: &'a Value, path: &str) -> &'a Value {
    config["storage"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == path)
        .unwrap_or_else(|| panic!("no {path}"))
}

fn file_text(config: &Value, path: &str) -> String {
    let source = file(config, path)["contents"]["source"].as_str().unwrap();
    base64_decode(source.strip_prefix("data:;base64,").unwrap()).unwrap()
}

fn unit<'a>(config: &'a Value, name: &str) -> &'a Value {
    config["systemd"]["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["name"] == name)
        .unwrap_or_else(|| panic!("no {name}"))
}

fn has_unit(config: &Value, name: &str) -> bool {
    config["systemd"]["units"]
        .as_array()
        .unwrap()
        .iter()
        .any(|unit| unit["name"] == name)
}

/// The `key=value` settings of unit `name`, in order.
fn unit_settings(config: &Value, name: &str) -> Vec<(String, String)> {
    unit(config, name)["contents"]
        .as_str()
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

fn setting(config: &Value, unit_name: &str, key: &str) -> String {
    unit_settings(config, unit_name)
        .into_iter()
        .find(|(name, _)| name == key)
        .unwrap_or_else(|| panic!("{unit_name} has no {key}"))
        .1
}

fn domain() -> NodeVmDomainTemplate {
    NodeVmDomainTemplate {
        name: "ceos1-abc12345".to_owned(),
        memory_mib: 6144,
        vcpus: 2,
        kernel: "/opt/sherpa/images/node_vm/flatcar-1.2.3.vmlinuz".to_owned(),
        cmdline: "root=LABEL=ROOT flatcar.first_boot=detected".to_owned(),
        disk: "/opt/sherpa/libvirt/images/ceos1-abc12345-hdd.qcow2".to_owned(),
        docker_disk: "/opt/sherpa/libvirt/images/ceos1-abc12345-docker.qcow2".to_owned(),
        image_archive: None,
        config_drive: "/opt/sherpa/libvirt/images/ceos1-abc12345.iso".to_owned(),
        management: Some(NodeVmManagement {
            network: "sherpa-management-abc12345".to_owned(),
            mac: "52:54:00:00:00:01".to_owned(),
        }),
        nics: vec![
            NodeVmNic {
                tap: "tpa1-abc12345".to_owned(),
                mac: "52:54:00:00:00:02".to_owned(),
                bridge: None,
            },
            NodeVmNic {
                tap: "tpb2-abc12345".to_owned(),
                mac: "52:54:00:00:00:03".to_owned(),
                bridge: None,
            },
            NodeVmNic {
                tap: "bra3-abc12345".to_owned(),
                mac: "52:54:00:00:00:04".to_owned(),
                bridge: Some("brs-lan-abc12345".to_owned()),
            },
        ],
        status_socket: "/opt/sherpa/run/console/abc12345/1.status".to_owned(),
        serial_socket: Some("/opt/sherpa/run/console/abc12345/1.serial".to_owned()),
    }
}

/// The domain XML with whitespace between elements removed, so elements read as one string.
fn compact(xml: &str) -> String {
    xml.lines().map(str::trim).collect()
}

// ============================================================================
// Domain
// ============================================================================

#[test]
fn node_vm_001_the_domain_boots_its_kernel_with_its_disks_and_the_ignition_config_disk() {
    let xml = compact(&domain().render().unwrap());
    assert!(xml.contains(
        "<kernel>/opt/sherpa/images/node_vm/flatcar-1.2.3.vmlinuz</kernel>\
         <cmdline>root=LABEL=ROOT flatcar.first_boot=detected</cmdline>"
    ));
    assert!(!xml.contains("<boot "));
    assert!(xml.contains("machine='q35'"));
    assert!(xml.contains(
        "<source file='/opt/sherpa/libvirt/images/ceos1-abc12345-hdd.qcow2'/>\
         <target dev='vda' bus='virtio'/>"
    ));
    assert!(xml.contains(
        "<source file='/opt/sherpa/libvirt/images/ceos1-abc12345-docker.qcow2'/>\
         <target dev='vdb' bus='virtio'/>"
    ));
    assert!(xml.contains(
        "<disk type='file' device='cdrom'><driver name='qemu' type='raw'/>\
         <source file='/opt/sherpa/libvirt/images/ceos1-abc12345.iso'/>\
         <target dev='sda' bus='sata'/><readonly/></disk>"
    ));
}

#[test]
fn node_vm_002_each_data_interface_is_a_tap_named_like_its_link_end() {
    let xml = compact(&domain().render().unwrap());
    assert!(xml.contains(
        "<interface type='ethernet'><mtu size='9600'/><mac address='52:54:00:00:00:02'/>\
         <target dev='tpa1-abc12345'/>"
    ));
    assert!(xml.contains("<target dev='tpb2-abc12345'/>"));
    assert_eq!(xml.matches("<interface ").count(), 4);
    assert!(xml.contains(
        "<interface type='network'><mac address='52:54:00:00:00:01'/>\
         <source network='sherpa-management-abc12345'/>"
    ));
}

#[test]
fn node_vm_056_a_bridge_member_is_a_bridge_interface_so_libvirt_enslaves_it_on_every_start() {
    let xml = compact(&domain().render().unwrap());
    assert!(xml.contains(
        "<interface type='bridge'><mtu size='9600'/><mac address='52:54:00:00:00:04'/>\
         <source bridge='brs-lan-abc12345'/><target dev='bra3-abc12345'/>"
    ));
    assert_eq!(xml.matches("<interface type='ethernet'>").count(), 2);
}

#[test]
fn node_vm_005_values_are_escaped() {
    let mut domain = domain();
    domain.name = "a<b".to_owned();
    let xml = domain.render().unwrap();
    assert!(!xml.contains("a<b"));
    assert!(xml.contains("<name>a&"));
}

#[test]
fn node_vm_031_a_docker_store_vm_has_no_nics_and_no_console_and_its_image_archive_read_only() {
    let mut domain = domain();
    domain.management = None;
    domain.nics.clear();
    domain.serial_socket = None;
    domain.image_archive = Some("/opt/sherpa/libvirt/images/flatcar-docker-0123.tar".to_owned());
    let xml = compact(&domain.render().unwrap());
    assert!(!xml.contains("<interface "));
    assert!(!xml.contains("<serial "));
    assert!(!xml.contains("<console "));
    assert!(xml.contains(
        "<driver name='qemu' type='raw'/>\
         <source file='/opt/sherpa/libvirt/images/flatcar-docker-0123.tar'/>\
         <target dev='vdc' bus='virtio'/><serial>sherpa-image</serial><readonly/>"
    ));
    assert!(xml.contains("<channel type='unix'>"));
}

#[test]
fn node_vm_048_the_vm_has_its_serial_console_on_the_nodes_socket_and_its_status_channel() {
    let xml = compact(&domain().render().unwrap());
    assert!(xml.contains(
        "<serial type='unix'><source mode='bind' path='/opt/sherpa/run/console/abc12345/1.serial'/>"
    ));
    assert!(xml.contains(
        "<channel type='unix'>\
         <source mode='bind' path='/opt/sherpa/run/console/abc12345/1.status'/>\
         <target type='virtio' name='org.sherpa.status'/></channel>"
    ));
    assert!(xml.contains("<model type='none'/>"));
    assert!(xml.contains("<memballoon model='none'/>"));
    assert!(!domain().render().unwrap().contains("<image_archive"));
}

// ============================================================================
// Ignition
// ============================================================================

#[test]
fn node_vm_008_updates_reboots_and_logins_are_off_in_the_node_vm() {
    for config in [node_config(), store_config()] {
        assert_eq!(
            file(&config, "/etc/flatcar/update.conf")["contents"]["source"],
            "data:,REBOOT_STRATEGY%3Doff%0A"
        );
        for name in [
            "update-engine.service",
            "locksmithd.service",
            "sshd.socket",
            "sshd-vsock.socket",
            "sshd.service",
            "serial-getty@ttyS0.service",
        ] {
            assert_eq!(unit(&config, name)["mask"], true, "{name}");
        }
        assert_eq!(config["passwd"]["users"], serde_json::json!([]));
    }
}

#[test]
fn node_vm_039_the_first_boot_does_not_rebuild_the_library_cache_or_hardware_database() {
    let config = node_config();
    for name in ["ldconfig.service", "systemd-hwdb-update.service"] {
        assert_eq!(unit(&config, name)["mask"], true, "{name}");
    }
}

#[test]
fn node_vm_040_docker_in_the_node_vm_sets_up_no_iptables_rules_and_listens_on_its_socket_only() {
    let options = file_text(
        &node_config(),
        "/etc/systemd/system/docker.service.d/10-sherpa.conf",
    );
    let opts = options
        .lines()
        .find_map(|line| line.strip_prefix(r#"Environment="DOCKER_OPTS="#))
        .unwrap()
        .trim_end_matches('"');
    assert_eq!(
        opts.split(' ').collect::<Vec<_>>(),
        ["--bridge=none", "--iptables=false", "--ip6tables=false"]
    );
}

#[test]
fn node_vm_041_containerd_in_the_node_vm_runs_without_the_cri_plugins() {
    let config = node_config();
    assert_eq!(
        file_text(
            &config,
            "/etc/systemd/system/containerd.service.d/10-sherpa.conf"
        ),
        "[Service]\nEnvironment=CONTAINERD_CONFIG=/etc/containerd/config.toml\n"
    );
    let containerd = file_text(&config, "/etc/containerd/config.toml");
    let disabled: Vec<&str> = containerd
        .lines()
        .skip_while(|line| !line.starts_with("disabled_plugins"))
        .take_while(|line| !line.starts_with(']'))
        .filter_map(|line| line.trim().strip_prefix('"')?.strip_suffix("\","))
        .collect();
    assert!(disabled.contains(&"io.containerd.grpc.v1.cri"));
    assert!(containerd.contains("address = \"/run/containerd/containerd.sock\""));
}

#[test]
fn node_vm_010_the_nics_are_named_by_their_lowercase_mac_before_docker_and_nothing_manages_them() {
    let config = node_config();
    assert_eq!(
        file_text(&config, "/etc/sherpa/interfaces"),
        "52:54:00:ab:00:01 eth0\n52:54:00:ab:00:02 eth1\n"
    );
    assert_eq!(
        setting(&config, "sherpa-links.service", "Before"),
        "docker.service"
    );
    assert_eq!(setting(&config, "sherpa-links.service", "Type"), "oneshot");
    assert_eq!(
        setting(&config, "sherpa-links.service", "ExecStart"),
        "/opt/sherpa/links"
    );
    assert_eq!(file(&config, "/opt/sherpa/links")["mode"], 0o755);
    for name in ["systemd-networkd.service", "systemd-networkd.socket"] {
        assert_eq!(unit(&config, name)["mask"], true, "{name}");
    }
    assert!(
        config["storage"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| !file["path"]
                .as_str()
                .unwrap()
                .starts_with("/etc/systemd/network/"))
    );
}

#[test]
fn node_vm_046_before_docker_the_guest_sets_the_management_addresses_and_default_routes() {
    assert_eq!(
        file_text(&node_config(), "/etc/sherpa/management"),
        "eth0 172.31.0.23/24 172.31.0.1\neth0 fd00:b0d1:0:1::17/64 fd00:b0d1:0:1::1\n"
    );
}

#[test]
fn node_vm_035_interfaces_on_no_link_are_listed_with_their_fixed_mac_for_the_guest_to_make() {
    assert_eq!(
        file_text(&node_config(), "/etc/sherpa/placeholders"),
        "52:54:00:ab:00:03 eth2\n52:54:00:ab:00:04 eth3\n"
    );
}

#[test]
fn node_vm_036_docker_keeps_its_data_on_the_store_disk_which_the_store_vm_formats() {
    let node = node_config();
    assert_eq!(
        setting(&node, "var-lib-docker.mount", "What"),
        "/dev/disk/by-label/sherpa-docker"
    );
    assert_eq!(
        setting(&node, "var-lib-docker.mount", "Where"),
        "/var/lib/docker"
    );
    assert_eq!(
        setting(&node, "var-lib-docker.mount", "RequiredBy"),
        "containerd.service docker.service"
    );
    assert_eq!(node["storage"]["filesystems"], serde_json::json!([]));
    let store = store_config();
    assert_eq!(store["storage"]["filesystems"][0]["device"], "/dev/vdb");
    assert_eq!(store["storage"]["filesystems"][0]["label"], "sherpa-docker");
    assert_eq!(unit(&store, "var-lib-docker.mount")["enabled"], true);
    for name in [
        "sherpa-links.service",
        "sherpa-node.service",
        "sherpa-console.service",
    ] {
        assert!(!has_unit(&store, name), "{name}");
    }
}

#[test]
fn node_vm_011_the_serial_console_runs_the_network_os_cli() {
    let config = node_config();
    assert_eq!(file_text(&config, "/etc/sherpa/console"), "Cli\n");
    assert_eq!(
        setting(&config, "sherpa-console.service", "ExecStart"),
        "/opt/sherpa/console"
    );
    assert_eq!(
        setting(&config, "sherpa-console.service", "TTYPath"),
        "/dev/ttyS0"
    );
}

#[test]
fn node_vm_049_a_unit_starts_the_container_after_docker_and_the_nics_from_sherpas_arguments() {
    let config = node_config();
    assert_eq!(
        file_text(&config, "/etc/sherpa/create"),
        "--name\0ceos1-abc12345\0--env\0A=b c\0ceos:4.34\0"
    );
    assert_eq!(
        file_text(&config, "/etc/sherpa/container"),
        "ceos1-abc12345\n"
    );
    let unit_name = "sherpa-node.service";
    assert_eq!(unit(&config, unit_name)["enabled"], true);
    assert_eq!(
        setting(&config, unit_name, "Requires"),
        "docker.service sherpa-links.service"
    );
    assert_eq!(
        setting(&config, unit_name, "After"),
        "docker.service sherpa-links.service"
    );
    assert_eq!(
        setting(&config, unit_name, "RequiresMountsFor"),
        NODE_VM_CONFIG_DRIVE_MOUNT
    );
    assert_eq!(setting(&config, unit_name, "ExecStart"), "/opt/sherpa/node");
    assert_eq!(file(&config, "/opt/sherpa/node")["mode"], 0o755);
}

#[test]
fn node_vm_050_the_config_drive_is_mounted_and_writable_volumes_are_listed_for_copying_in() {
    let config = node_config();
    assert_eq!(
        setting(&config, "media-config.mount", "What"),
        "/dev/disk/by-label/config-2"
    );
    assert_eq!(
        setting(&config, "media-config.mount", "Where"),
        NODE_VM_CONFIG_DRIVE_MOUNT
    );
    assert_eq!(setting(&config, "media-config.mount", "Options"), "ro");
    assert_eq!(
        file_text(&config, "/etc/sherpa/writable"),
        "/media/config/sherpa/volumes/1 /etc/opt/srlinux/config.json\n"
    );
}

#[test]
fn node_vm_051_both_vms_have_the_status_report_and_the_store_vm_loads_its_archive() {
    let node = node_config();
    let store = store_config();
    for config in [&node, &store] {
        assert_eq!(file(config, "/opt/sherpa/report")["mode"], 0o755);
    }
    assert_eq!(
        setting(&store, "sherpa-store.service", "After"),
        "docker.service"
    );
    assert_eq!(
        setting(&store, "sherpa-store.service", "ExecStart"),
        "/opt/sherpa/store"
    );
    assert_eq!(unit(&store, "sherpa-store.service")["enabled"], true);
    assert!(!has_unit(&node, "sherpa-store.service"));
}
