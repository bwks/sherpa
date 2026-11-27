use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use askama::Template;
use serde_json::Value;
use tempfile::TempDir;

use shared::util::base64_decode;
use template::{
    GuestAddress, GuestInterface, NodeScript, SANDBOX_CONFIG_DRIVE_MOUNT, SandboxDomainTemplate,
    SandboxGuest, SandboxManagement, SandboxNic, WritableVolume, store_vm_ignition_json,
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
    let json = SandboxGuest {
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

/// The `(section, key, value)` settings of unit file or drop-in `text`, in order, as systemd
/// reads them.
fn ini(text: &str) -> Vec<(String, String, String)> {
    let mut section = String::new();
    let mut settings = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            name.clone_into(&mut section);
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("not a setting: {line}"));
        settings.push((section.clone(), key.to_owned(), value.to_owned()));
    }
    settings
}

/// The value of `key` in `section` of unit file or drop-in `text`.
fn ini_value(text: &str, section: &str, key: &str) -> String {
    ini(text)
        .into_iter()
        .find(|(name, setting, _)| name == section && setting == key)
        .unwrap_or_else(|| panic!("no {key} in [{section}]"))
        .2
}

fn setting(config: &Value, unit_name: &str, section: &str, key: &str) -> String {
    ini_value(
        unit(config, unit_name)["contents"].as_str().unwrap(),
        section,
        key,
    )
}

/// A stand-in for Docker in the guest: it logs each command, has a container once `create` has
/// made it until `rm` removes it, keeps what `cp` reads, and fails `cp` while `FAIL_CP` is set.
const FAKE_DOCKER: &str = r#"#!/bin/bash
echo "$*" >> "$FAKE_GUEST/docker.log"
case "$1" in
  container) [ -e "$FAKE_GUEST/container" ] ;;
  create) touch "$FAKE_GUEST/container" ;;
  rm) rm -f "$FAKE_GUEST/container" ;;
  cp)
    cat > "$FAKE_GUEST/copied.tar"
    if [ -n "${FAIL_CP:-}" ]; then
      echo "no space left on device" >&2
      exit 1
    fi
    ;;
esac
"#;

/// A stand-in for the status report: it keeps each status line.
const FAKE_REPORT: &str = r#"#!/bin/bash
echo "$*" >> "$FAKE_GUEST/reports"
"#;

/// The node script, run in a folder of its own with Docker and the report stubbed, as a node
/// VM's boots run it.
struct FakeGuest {
    dir: TempDir,
}

impl FakeGuest {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let path = dir.path();
        fs::create_dir(path.join("bin")).unwrap();
        fs::write(path.join("container-name"), "srl1-abc12345\n").unwrap();
        fs::write(path.join("create"), "--name\0srl1-abc12345\0srl:26\0").unwrap();
        fs::write(path.join("config.json"), "{}").unwrap();
        fs::write(
            path.join("writable"),
            format!(
                "{} /etc/opt/srlinux/config.json\n",
                path.join("config.json").display()
            ),
        )
        .unwrap();
        let guest = Self { dir };
        guest.executable("bin/docker", FAKE_DOCKER);
        guest.executable("report", FAKE_REPORT);
        let script = NodeScript {
            container: &guest.path("container-name"),
            create: &guest.path("create"),
            writable: &guest.path("writable"),
            report: &guest.path("report"),
            created: &guest.path("state/created"),
        }
        .render()
        .unwrap();
        guest.executable("node", &script);
        guest
    }

    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).display().to_string()
    }

    fn executable(&self, name: &str, contents: &str) {
        let path: PathBuf = self.dir.path().join(name);
        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Boot once: run the script, with `cp` failing when `fail_cp`, and return whether it
    /// succeeded.
    fn boot(&self, fail_cp: bool) -> bool {
        fs::write(self.dir.path().join("docker.log"), "").unwrap();
        let mut command = Command::new("bash");
        command
            .arg(self.path("node"))
            .env("FAKE_GUEST", self.dir.path())
            .env("PATH", format!("{}:/usr/bin:/bin", self.path("bin")));
        if fail_cp {
            command.env("FAIL_CP", "1");
        }
        command.status().unwrap().success()
    }

    fn docker_commands(&self) -> Vec<String> {
        fs::read_to_string(self.dir.path().join("docker.log"))
            .unwrap()
            .lines()
            .map(|line| line.split(' ').take(2).collect::<Vec<_>>().join(" "))
            .collect()
    }

    fn reports(&self) -> Vec<String> {
        fs::read_to_string(self.dir.path().join("reports"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn has(&self, name: &str) -> bool {
        Path::new(&self.path(name)).exists()
    }
}

fn domain() -> SandboxDomainTemplate {
    SandboxDomainTemplate {
        name: "ceos1-abc12345".to_owned(),
        memory_mib: 6144,
        vcpus: 2,
        kernel: "/opt/sherpa/images/sandbox/flatcar-1.2.3.vmlinuz".to_owned(),
        cmdline: "root=LABEL=ROOT flatcar.first_boot=detected".to_owned(),
        disk: "/opt/sherpa/libvirt/images/ceos1-abc12345-hdd.qcow2".to_owned(),
        docker_disk: "/opt/sherpa/libvirt/images/ceos1-abc12345-docker.qcow2".to_owned(),
        image_archive: None,
        config_drive: "/opt/sherpa/libvirt/images/ceos1-abc12345.iso".to_owned(),
        management: Some(SandboxManagement {
            network: "sherpa-management-abc12345".to_owned(),
            mac: "52:54:00:00:00:01".to_owned(),
        }),
        nics: vec![
            SandboxNic {
                tap: "tpa1-abc12345".to_owned(),
                mac: "52:54:00:00:00:02".to_owned(),
                bridge: None,
            },
            SandboxNic {
                tap: "tpb2-abc12345".to_owned(),
                mac: "52:54:00:00:00:03".to_owned(),
                bridge: None,
            },
            SandboxNic {
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
fn sandbox_001_the_domain_boots_its_kernel_with_its_disks_and_the_ignition_config_disk() {
    let xml = compact(&domain().render().unwrap());
    assert!(xml.contains(
        "<kernel>/opt/sherpa/images/sandbox/flatcar-1.2.3.vmlinuz</kernel>\
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
fn sandbox_002_each_data_interface_is_a_tap_named_like_its_link_end() {
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
fn sandbox_056_a_bridge_member_is_a_bridge_interface_so_libvirt_enslaves_it_on_every_start() {
    let xml = compact(&domain().render().unwrap());
    assert!(xml.contains(
        "<interface type='bridge'><mtu size='9600'/><mac address='52:54:00:00:00:04'/>\
         <source bridge='brs-lan-abc12345'/><target dev='bra3-abc12345'/>"
    ));
    assert_eq!(xml.matches("<interface type='ethernet'>").count(), 2);
}

#[test]
fn sandbox_005_values_are_escaped() {
    let mut domain = domain();
    domain.name = "a<b".to_owned();
    let xml = domain.render().unwrap();
    assert!(!xml.contains("a<b"));
    assert!(xml.contains("<name>a&"));
}

#[test]
fn sandbox_031_a_docker_store_vm_has_no_nics_and_no_console_and_its_image_archive_read_only() {
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
fn sandbox_048_the_vm_has_its_serial_console_on_the_nodes_socket_and_its_status_channel() {
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
fn sandbox_008_updates_reboots_and_logins_are_off_in_the_sandbox() {
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
fn sandbox_039_the_first_boot_does_not_rebuild_the_library_cache_or_hardware_database() {
    let config = node_config();
    for name in ["ldconfig.service", "systemd-hwdb-update.service"] {
        assert_eq!(unit(&config, name)["mask"], true, "{name}");
    }
}

#[test]
fn sandbox_040_docker_in_the_sandbox_sets_up_no_iptables_rules_and_listens_on_its_socket_only() {
    let options = file_text(
        &node_config(),
        "/etc/systemd/system/docker.service.d/10-sherpa.conf",
    );
    let environment = ini_value(&options, "Service", "Environment");
    let opts = environment
        .trim_matches('"')
        .strip_prefix("DOCKER_OPTS=")
        .unwrap();
    assert_eq!(
        opts.split(' ').collect::<Vec<_>>(),
        ["--bridge=none", "--iptables=false", "--ip6tables=false"]
    );
}

#[test]
fn sandbox_041_containerd_in_the_sandbox_runs_without_the_cri_plugins() {
    let config = node_config();
    let options = file_text(
        &config,
        "/etc/systemd/system/containerd.service.d/10-sherpa.conf",
    );
    assert_eq!(
        ini_value(&options, "Service", "Environment"),
        "CONTAINERD_CONFIG=/etc/containerd/config.toml"
    );
    let containerd: toml::Table = file_text(&config, "/etc/containerd/config.toml")
        .parse()
        .unwrap();
    let disabled = containerd["disabled_plugins"].as_array().unwrap();
    assert!(disabled.contains(&toml::Value::from("io.containerd.grpc.v1.cri")));
    assert_eq!(
        containerd["grpc"]["address"].as_str(),
        Some("/run/containerd/containerd.sock")
    );
}

#[test]
fn sandbox_010_the_nics_are_named_by_their_lowercase_mac_before_docker_and_nothing_manages_them() {
    let config = node_config();
    assert_eq!(
        file_text(&config, "/etc/sherpa/interfaces"),
        "52:54:00:ab:00:01 eth0\n52:54:00:ab:00:02 eth1\n"
    );
    assert_eq!(
        setting(&config, "sherpa-links.service", "Unit", "Before"),
        "docker.service"
    );
    assert_eq!(
        setting(&config, "sherpa-links.service", "Service", "Type"),
        "oneshot"
    );
    assert_eq!(
        setting(&config, "sherpa-links.service", "Service", "ExecStart"),
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
fn sandbox_046_before_docker_the_guest_sets_the_management_addresses_and_default_routes() {
    assert_eq!(
        file_text(&node_config(), "/etc/sherpa/management"),
        "eth0 172.31.0.23/24 172.31.0.1\neth0 fd00:b0d1:0:1::17/64 fd00:b0d1:0:1::1\n"
    );
}

#[test]
fn sandbox_035_interfaces_on_no_link_are_listed_with_their_fixed_mac_for_the_guest_to_make() {
    assert_eq!(
        file_text(&node_config(), "/etc/sherpa/placeholders"),
        "52:54:00:ab:00:03 eth2\n52:54:00:ab:00:04 eth3\n"
    );
}

#[test]
fn sandbox_036_docker_keeps_its_data_on_the_store_disk_which_the_store_vm_formats() {
    let node = node_config();
    assert_eq!(
        setting(&node, "var-lib-docker.mount", "Mount", "What"),
        "/dev/disk/by-label/sherpa-docker"
    );
    assert_eq!(
        setting(&node, "var-lib-docker.mount", "Mount", "Where"),
        "/var/lib/docker"
    );
    assert_eq!(
        setting(&node, "var-lib-docker.mount", "Install", "RequiredBy"),
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
fn sandbox_011_the_serial_console_runs_the_network_os_cli() {
    let config = node_config();
    assert_eq!(file_text(&config, "/etc/sherpa/console"), "Cli\n");
    assert_eq!(
        setting(&config, "sherpa-console.service", "Service", "ExecStart"),
        "/opt/sherpa/console"
    );
    assert_eq!(
        setting(&config, "sherpa-console.service", "Service", "TTYPath"),
        "/dev/ttyS0"
    );
}

#[test]
fn sandbox_049_a_unit_starts_the_container_after_docker_and_the_nics_from_sherpas_arguments() {
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
        setting(&config, unit_name, "Unit", "Requires"),
        "docker.service sherpa-links.service"
    );
    assert_eq!(
        setting(&config, unit_name, "Unit", "After"),
        "docker.service sherpa-links.service"
    );
    assert_eq!(
        setting(&config, unit_name, "Unit", "RequiresMountsFor"),
        SANDBOX_CONFIG_DRIVE_MOUNT
    );
    assert_eq!(
        setting(&config, unit_name, "Service", "ExecStart"),
        "/opt/sherpa/node"
    );
    assert_eq!(file(&config, "/opt/sherpa/node")["mode"], 0o755);
}

#[test]
fn sandbox_050_the_config_drive_is_mounted_and_writable_volumes_are_listed_for_copying_in() {
    let config = node_config();
    assert_eq!(
        setting(&config, "media-config.mount", "Mount", "What"),
        "/dev/disk/by-label/config-2"
    );
    assert_eq!(
        setting(&config, "media-config.mount", "Mount", "Where"),
        SANDBOX_CONFIG_DRIVE_MOUNT
    );
    assert_eq!(
        setting(&config, "media-config.mount", "Mount", "Options"),
        "ro"
    );
    assert_eq!(
        file_text(&config, "/etc/sherpa/writable"),
        "/media/config/sherpa/volumes/1 /etc/opt/srlinux/config.json\n"
    );
}

#[test]
fn sandbox_051_both_vms_have_the_status_report_and_the_store_vm_loads_its_archive() {
    let node = node_config();
    let store = store_config();
    for config in [&node, &store] {
        assert_eq!(file(config, "/opt/sherpa/report")["mode"], 0o755);
    }
    assert_eq!(
        setting(&store, "sherpa-store.service", "Unit", "After"),
        "docker.service"
    );
    assert_eq!(
        setting(&store, "sherpa-store.service", "Service", "ExecStart"),
        "/opt/sherpa/store"
    );
    assert_eq!(unit(&store, "sherpa-store.service")["enabled"], true);
    assert!(!has_unit(&node, "sherpa-store.service"));
}

// ============================================================================
// Node script
// ============================================================================

#[test]
fn sandbox_057_a_copy_into_the_container_that_fails_removes_the_container_and_reports_why() {
    let guest = FakeGuest::new();
    assert!(!guest.boot(true));
    assert!(!guest.has("container"));
    assert!(!guest.has("state/created"));
    assert!(
        !guest
            .docker_commands()
            .contains(&"start srl1-abc12345".to_owned())
    );
    let reports = guest.reports();
    assert_eq!(reports.len(), 1);
    assert!(
        reports[0].starts_with("failed: cannot copy the writable volumes into the container: ")
            && reports[0].contains("no space left on device"),
        "{reports:?}"
    );
}

#[test]
fn sandbox_058_a_container_a_first_boot_left_without_its_volumes_is_created_again_with_them() {
    let guest = FakeGuest::new();
    fs::write(guest.path("container"), "").unwrap();
    assert!(guest.boot(false));
    assert_eq!(
        guest.docker_commands(),
        [
            "container inspect",
            "rm -f",
            "create --name",
            "cp -",
            "start srl1-abc12345"
        ]
    );
    let listing = Command::new("tar")
        .arg("-tf")
        .arg(guest.path("copied.tar"))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(listing.stdout).unwrap(),
        "./etc/opt/srlinux/config.json\n"
    );
    assert!(guest.has("state/created"));
    assert_eq!(guest.reports(), ["started"]);
}

#[test]
fn sandbox_059_a_boot_after_the_container_was_made_only_starts_it() {
    let guest = FakeGuest::new();
    assert!(guest.boot(false));
    assert!(guest.boot(false));
    assert_eq!(guest.docker_commands(), ["start srl1-abc12345"]);
    assert_eq!(guest.reports(), ["started", "started"]);
}
