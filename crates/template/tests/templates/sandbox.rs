use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use askama::Template;
use serde_json::Value;
use tempfile::TempDir;
use toml::Value as TomlValue;

use shared::data::{
    ConnectionTypes, DiskCache, DiskIo, Interface, InterfaceType, NodeBootDisk, NodeCpu,
    NodeInterfaces, NodeNuma,
};
use shared::util::base64_decode;
use template::{
    DomainTemplate, DomainTuning, FlatcarDomain, GuestAddress, GuestInterface, NodeScript,
    SANDBOX_CONFIG_DRIVE_MOUNT, SANDBOX_CONTAINER_BODY, SandboxDomain, SandboxGuest, SandboxOwnNic,
    WritableVolume, boot_vm_ignition_json, store_vm_ignition_json,
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
        own_nic: &SandboxOwnNic {
            mac: "52:54:00:AB:00:09".to_owned(),
            addresses: vec![
                GuestAddress {
                    address: "172.31.0.30".parse().unwrap(),
                    prefix_length: 24,
                    gateway: "172.31.0.1".parse().unwrap(),
                },
                GuestAddress {
                    address: "fd00:b0d1:0:1::1e".parse().unwrap(),
                    prefix_length: 64,
                    gateway: "fd00:b0d1:0:1::1".parse().unwrap(),
                },
            ],
        },
        console_command: &["Cli"],
        docker_api_version: "1.49",
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

/// A stand-in for curl talking to the guest's Docker, which logs each request's method and URL,
/// keeps what a create or an upload sends, has a container from a create until a `DELETE`, and
/// answers as Docker does, refusing a create or an upload with a message while `FAIL_CREATE` or
/// `FAIL_ARCHIVE` is set and answering a start as done while `RUNNING` is set.
const FAKE_CURL: &str = r#"#!/bin/bash
while [ $# -gt 0 ]; do
  case "$1" in
    --request) method=$2; shift 2 ;;
    --output) output=$2; shift 2 ;;
    --unix-socket) echo "$2" >> "$FAKE_GUEST/sockets"; shift 2 ;;
    --data-binary) data=$2; shift 2 ;;
    --header|--write-out) shift 2 ;;
    --*) shift ;;
    *) url=$1; shift ;;
  esac
done
echo "$method $url" >> "$FAKE_GUEST/requests"
answer() {
  printf '%s' "$2" > "$output"
  printf '%s' "$1"
}
case "$method $url" in
  "DELETE "*)
    if [ -e "$FAKE_GUEST/container" ]; then
      rm "$FAKE_GUEST/container"
      answer 204 ""
    else
      answer 404 '{"message":"No such container"}'
    fi
    ;;
  "POST "*/containers/create*)
    if [ -n "${FAIL_CREATE:-}" ]; then
      answer 404 '{"message":"No such image: srl:26"}'
    else
      cp "${data#@}" "$FAKE_GUEST/created-from.json"
      touch "$FAKE_GUEST/container"
      answer 201 '{"Id":"0123"}'
    fi
    ;;
  "PUT "*/archive*)
    cat > "$FAKE_GUEST/copied.tar"
    if [ -n "${FAIL_ARCHIVE:-}" ]; then
      answer 500 '{"message":"no space left on device"}'
    else
      answer 200 ""
    fi
    ;;
  "POST "*/start)
    if [ -n "${RUNNING:-}" ]; then answer 304 ""; else answer 204 ""; fi
    ;;
esac
"#;

/// A stand-in for the status report: it keeps each status line.
const FAKE_REPORT: &str = r#"#!/bin/bash
echo "$*" >> "$FAKE_GUEST/reports"
"#;

/// The Engine API version the fake guest's script is given.
const API_VERSION: &str = "1.49";

/// The node script, run in a folder of its own with curl and the report stubbed, as a node VM's
/// boots run it.
struct FakeGuest {
    dir: TempDir,
}

impl FakeGuest {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let path = dir.path();
        fs::create_dir(path.join("bin")).unwrap();
        fs::write(path.join("container-name"), "srl1-abc12345\n").unwrap();
        fs::write(path.join("container.json"), r#"{"Image":"srl:26"}"#).unwrap();
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
        guest.executable("bin/curl", FAKE_CURL);
        guest.executable("report", FAKE_REPORT);
        let script = NodeScript {
            api_version: API_VERSION.to_owned(),
            socket: guest.path("docker.sock"),
            container: guest.path("container-name"),
            body: guest.path("container.json"),
            writable: guest.path("writable"),
            report: guest.path("report"),
            created: guest.path("state/created"),
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

    /// Boot once with the fake Docker's settings `env`, and return whether the script
    /// succeeded.
    fn boot(&self, env: &[&str]) -> bool {
        fs::write(self.dir.path().join("requests"), "").unwrap();
        let mut command = Command::new("bash");
        command
            .arg(self.path("node"))
            .env("FAKE_GUEST", self.dir.path())
            .env("PATH", format!("{}:/usr/bin:/bin", self.path("bin")));
        for name in env {
            command.env(name, "1");
        }
        command.status().unwrap().success()
    }

    /// The requests of the last boot, each its method and URL.
    fn requests(&self) -> Vec<String> {
        self.lines("requests")
    }

    /// The requests of the last boot, each its method and the path after the API version.
    fn api_calls(&self) -> Vec<String> {
        let prefix = format!("http://docker/v{API_VERSION}");
        self.requests()
            .iter()
            .map(|request| {
                let (method, url) = request.split_once(' ').unwrap();
                format!("{method} {}", url.strip_prefix(&prefix).unwrap_or(url))
            })
            .collect()
    }

    fn reports(&self) -> Vec<String> {
        self.lines("reports")
    }

    fn lines(&self, name: &str) -> Vec<String> {
        fs::read_to_string(self.dir.path().join(name))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn has(&self, name: &str) -> bool {
        Path::new(&self.path(name)).exists()
    }
}

fn domain_interface(
    name: &str,
    mac: &str,
    connection_type: ConnectionTypes,
    tap: Option<&str>,
) -> Interface {
    Interface {
        name: name.to_owned(),
        num: 0,
        mac_address: mac.to_owned(),
        mtu: if connection_type == ConnectionTypes::Management {
            1500
        } else {
            9600
        },
        connection_type,
        tap: tap.map(str::to_owned),
    }
}

fn flatcar_vm(name: &str, disk: &str, config_drive: &str) -> FlatcarDomain {
    FlatcarDomain {
        name: name.to_owned(),
        memory: 2048,
        vcpus: 2,
        qemu_bin: "/usr/bin/qemu-system-x86_64".to_owned(),
        disk: disk.to_owned(),
        config_drive: config_drive.to_owned(),
    }
}

fn sandbox_domain(tuning: DomainTuning) -> DomainTemplate {
    FlatcarDomain {
        name: "ceos1-abc12345".to_owned(),
        memory: 6144,
        vcpus: 2,
        qemu_bin: "/usr/bin/qemu-system-x86_64".to_owned(),
        disk: "/opt/sherpa/libvirt/images/ceos1-abc12345-hdd.qcow2".to_owned(),
        config_drive: "/opt/sherpa/libvirt/images/ceos1-abc12345.iso".to_owned(),
    }
    .sandbox(SandboxDomain {
        docker_disk: "/opt/sherpa/libvirt/images/ceos1-abc12345-docker.qcow2".to_owned(),
        lab_id: "abc12345".to_owned(),
        management_network: "sherpa-management-abc12345".to_owned(),
        interfaces: vec![
            domain_interface(
                "eth0",
                "52:54:00:00:00:01",
                ConnectionTypes::Management,
                None,
            ),
            domain_interface(
                "tpa1-abc12345",
                "52:54:00:00:00:02",
                ConnectionTypes::P2p,
                None,
            ),
            domain_interface(
                "tpb2-abc12345",
                "52:54:00:00:00:03",
                ConnectionTypes::P2p,
                None,
            ),
            domain_interface(
                "brs-lan-abc12345",
                "52:54:00:00:00:04",
                ConnectionTypes::PrivateBridge,
                Some("bra3-abc12345"),
            ),
        ],
        own_mac: "52:54:00:00:00:09".to_owned(),
        serial_socket: "/opt/sherpa/run/console/abc12345/1.serial".to_owned(),
        status_socket: "/opt/sherpa/run/console/abc12345/1.status".to_owned(),
        tuning,
    })
}

fn domain() -> DomainTemplate {
    sandbox_domain(DomainTuning::default())
}

fn store_vm_domain() -> DomainTemplate {
    flatcar_vm(
        "sherpa-store-0123",
        "/opt/sherpa/libvirt/images/flatcar-docker-0123-vm.qcow2",
        "/opt/sherpa/libvirt/images/flatcar-docker-0123.iso",
    )
    .store_vm(
        "/opt/sherpa/libvirt/images/flatcar-docker-0123.qcow2.part".to_owned(),
        "/opt/sherpa/libvirt/images/flatcar-docker-0123.tar".to_owned(),
        "/opt/sherpa/run/sandbox/flatcar-docker-0123.status".to_owned(),
    )
}

fn boot_vm_domain() -> DomainTemplate {
    flatcar_vm(
        "sherpa-flatcar-sandbox.4757.2.4",
        "/opt/sherpa/libvirt/images/flatcar-sandbox.4757.2.4.qcow2.part",
        "/opt/sherpa/libvirt/images/flatcar-sandbox.4757.2.4.iso",
    )
    .boot_vm()
}

/// The domain XML with whitespace between elements removed, so elements read as one string.
fn compact(xml: &str) -> String {
    xml.lines().map(str::trim).collect()
}

// ============================================================================
// Domain
// ============================================================================

#[test]
fn sandbox_001_the_domain_boots_its_disk_with_its_docker_disk_and_the_ignition_config_disk() {
    let xml = compact(&domain().render().unwrap());
    assert!(xml.contains("<type arch='x86_64' machine='q35'>hvm</type>"));
    assert!(xml.contains("<cpu mode='host-passthrough'>"));
    assert!(!xml.contains("<kernel>"));
    assert!(!xml.contains("<loader"));
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
        "<interface type='ethernet'><alias name='ua-net-ceos1-abc12345-p2p-tpa1-abc12345'/>\
         <mtu size='9600'/><mac address='52:54:00:00:00:02'/><target dev='tpa1-abc12345'/>\
         <script path=''/><model type='virtio'/>"
    ));
    assert!(xml.contains("<target dev='tpb2-abc12345'/>"));
    assert_eq!(xml.matches("<interface ").count(), 5);
    assert!(xml.contains(
        "<interface type='network'><alias name='ua-net-ceos1-abc12345-mgmt-eth0'/>\
         <mtu size='1500'/><mac address='52:54:00:00:00:01'/>\
         <source network='sherpa-management-abc12345'/><model type='virtio'/></interface>"
    ));
}

#[test]
fn sandbox_070_the_sandbox_has_a_nic_of_its_own_on_the_management_network() {
    let xml = compact(&domain().render().unwrap());
    assert!(xml.contains(
        "<interface type='network'><alias name='ua-net-ceos1-abc12345-sandbox'/>\
         <mac address='52:54:00:00:00:09'/><source network='sherpa-management-abc12345'/>\
         <model type='virtio'/></interface>"
    ));
}

#[test]
fn sandbox_071_before_its_other_nics_are_named_the_guest_moves_its_own_nic_out_of_the_network_oss_namespace()
 {
    let config = node_config();
    assert_eq!(
        file_text(&config, "/etc/sherpa/sandbox-nic"),
        "52:54:00:ab:00:09\n172.31.0.30/24 172.31.0.1\nfd00:b0d1:0:1::1e/64 fd00:b0d1:0:1::1\n"
    );
    let links = file_text(&config, "/opt/sherpa/links");
    let moved = links
        .find(r#"ip link set dev "$dev" netns "$SANDBOX_NETNS""#)
        .unwrap();
    assert!(moved < links.find(r#"name "sln$index""#).unwrap());
    assert!(links.contains("SANDBOX_NETNS=sherpa-sandbox\n"));
    assert!(links.contains(r#"ip -n "$SANDBOX_NETNS" addr replace "$address" dev sbx0"#));
    assert!(
        links.contains(r#"ip -n "$SANDBOX_NETNS" route replace default via "$gateway" dev sbx0"#)
    );
    assert!(links.contains("done\n} < /etc/sherpa/sandbox-nic\n"));
}

#[test]
fn sandbox_079_the_guest_turns_off_router_advertisements_on_the_sandboxs_own_nic_before_it_is_up() {
    let links = file_text(&node_config(), "/opt/sherpa/links");
    let off = links
        .find(
            r#"ip netns exec "$SANDBOX_NETNS" sysctl -q -w \
    net.ipv6.conf.sbx0.accept_ra=0 \
    net.ipv6.conf.sbx0.autoconf=0
"#,
        )
        .unwrap();
    assert!(
        off < links
            .find(r#"ip -n "$SANDBOX_NETNS" link set dev sbx0 up"#)
            .unwrap()
    );
    assert_eq!(links.matches("accept_ra").count(), 1);
    assert_eq!(links.matches("autoconf").count(), 1);
}

#[test]
fn sandbox_056_a_bridge_member_is_a_bridge_interface_so_libvirt_enslaves_it_on_every_start() {
    let xml = compact(&domain().render().unwrap());
    assert!(xml.contains(
        "<interface type='bridge'><alias name='ua-net-ceos1-abc12345-shared-bridge-brs-lan-abc12345'/>\
         <mac address='52:54:00:00:00:04'/><source bridge='brs-lan-abc12345'/>\
         <mtu size='9600'/><target dev='bra3-abc12345'/><model type='virtio'/>"
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
    let xml = compact(&store_vm_domain().render().unwrap());
    assert!(!xml.contains("<interface "));
    assert!(!xml.contains("<serial "));
    assert!(!xml.contains("<console "));
    assert!(xml.contains(
        "<driver name='qemu' type='raw'/>\
         <source file='/opt/sherpa/libvirt/images/flatcar-docker-0123.tar'/>\
         <target dev='vdc' bus='virtio'/><serial>sherpa-image</serial><readonly/>"
    ));
    assert!(xml.contains(
        "<channel type='unix'>\
         <source mode='bind' path='/opt/sherpa/run/sandbox/flatcar-docker-0123.status'/>\
         <target type='virtio' name='org.sherpa.status'/></channel>"
    ));
}

#[test]
fn sandbox_076_the_boot_disk_vm_sets_only_the_kernel_arguments_and_its_first_reboot_stops_it() {
    let config: Value = serde_json::from_str(&boot_vm_ignition_json().unwrap()).unwrap();
    assert_eq!(
        config["kernelArguments"]["shouldExist"],
        serde_json::json!(["quiet"])
    );
    assert_eq!(config["storage"]["files"], serde_json::json!([]));
    assert_eq!(config["systemd"]["units"], serde_json::json!([]));
    assert_eq!(config["passwd"]["users"], serde_json::json!([]));
    assert!(node_config().get("kernelArguments").is_none());

    let xml = compact(&boot_vm_domain().render().unwrap());
    assert!(xml.contains("<on_reboot>destroy</on_reboot>"));
    assert!(
        compact(&domain().render().unwrap()).contains("<on_reboot>restart</on_reboot>"),
        "a sandbox's reboot restarts it"
    );
    assert!(xml.contains(
        "<source file='/opt/sherpa/libvirt/images/flatcar-sandbox.4757.2.4.qcow2.part'/>\
         <target dev='vda' bus='virtio'/>"
    ));
    assert!(xml.contains(
        "<source file='/opt/sherpa/libvirt/images/flatcar-sandbox.4757.2.4.iso'/>\
         <target dev='sda' bus='sata'/><readonly/>"
    ));
    for absent in [
        "<interface ",
        "<serial ",
        "<console ",
        "<channel ",
        "<kernel>",
    ] {
        assert!(!xml.contains(absent), "{absent}");
    }
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
    assert!(!xml.contains("sherpa-image"));
}

#[test]
fn sandbox_100_a_sandboxs_domain_carries_the_tuning_its_node_gives_it() {
    let tuning = DomainTuning::from_node_settings(
        &NodeCpu {
            pinning: Some(vec![4, 5]),
            emulator_pinning: Some(vec![6]),
            ..NodeCpu::default()
        },
        &NodeNuma {
            node: Some(0),
            memory_mode: None,
        },
        &NodeBootDisk {
            cache: Some(DiskCache::None),
            io: Some(DiskIo::Native),
            io_thread_cpus: Some(vec![7]),
            ..NodeBootDisk::default()
        },
        &NodeInterfaces {
            data_count: None,
            queues: Some(2),
        },
        &InterfaceType::Virtio,
    );
    let xml = compact(&sandbox_domain(tuning).render().unwrap());
    assert!(xml.contains(
        "<cputune><vcpupin vcpu='0' cpuset='4'/><vcpupin vcpu='1' cpuset='5'/>\
         <emulatorpin cpuset='6'/><iothreadpin iothread='1' cpuset='7'/></cputune>"
    ));
    assert!(xml.contains("<numatune><memory mode='strict' nodeset='0'/></numatune>"));
    assert!(xml.contains(
        "<driver name='qemu' type='qcow2' cache='none' io='native' iothread='1'/>\
         <source file='/opt/sherpa/libvirt/images/ceos1-abc12345-hdd.qcow2'/>"
    ));
    // The management NIC and each data NIC take the queues; the sandbox's own NIC none.
    assert_eq!(xml.matches("<driver queues='2'/>").count(), 4);
    let own = &xml[xml
        .find("<alias name='ua-net-ceos1-abc12345-sandbox'/>")
        .unwrap()..];
    assert!(!own[..own.find("</interface>").unwrap()].contains("queues"));

    let untuned = compact(&domain().render().unwrap());
    for absent in [
        "<cputune>",
        "<numatune>",
        "<iothreads>",
        "cache=",
        "queues=",
    ] {
        assert!(!untuned.contains(absent), "{absent}");
    }
}

#[test]
fn sandbox_101_the_sandbox_store_and_boot_vms_have_no_vnc_guest_agent_usb_balloon_or_nocloud() {
    for domain in [domain(), store_vm_domain(), boot_vm_domain()] {
        let xml = compact(&domain.render().unwrap());
        for absent in [
            "<graphics",
            "org.qemu.guest_agent.0",
            "qemu-xhci",
            "ds=nocloud",
            "<smbios",
        ] {
            assert!(!xml.contains(absent), "{}: {absent}", domain.name);
        }
        assert!(
            xml.contains("<video><model type='none'/></video>"),
            "{}",
            domain.name
        );
        assert!(
            xml.contains("<memballoon model='none'/>"),
            "{}",
            domain.name
        );
        // Without a USB controller of model none, libvirt adds one of its own.
        assert!(
            xml.contains("<controller type='usb' model='none'/>"),
            "{}",
            domain.name
        );
    }
}

#[test]
fn sandbox_102_the_sandbox_store_and_boot_vms_boot_their_disk_only_never_the_config_drive() {
    for domain in [domain(), store_vm_domain(), boot_vm_domain()] {
        let xml = compact(&domain.render().unwrap());
        assert!(
            xml.contains(
                "<bootmenu enable='no'/><boot dev='hd'/><!-- SeaBios - No stanza required --></os>"
            ),
            "{}",
            domain.name
        );
        assert!(!xml.contains("<boot dev='cdrom'/>"), "{}", domain.name);
        assert!(!xml.contains("<loader"), "{}", domain.name);
    }
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
    assert!(disabled.contains(&TomlValue::from("io.containerd.grpc.v1.cri")));
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
fn sandbox_049_a_unit_starts_the_container_after_docker_and_the_nics_from_the_body_on_the_config_drive()
 {
    let config = node_config();
    assert_eq!(
        file_text(&config, "/etc/sherpa/container"),
        "ceos1-abc12345\n"
    );
    assert!(file_text(&config, "/opt/sherpa/node").contains(&format!(
        "@{SANDBOX_CONFIG_DRIVE_MOUNT}/{SANDBOX_CONTAINER_BODY}"
    )));
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
    assert!(!guest.boot(&["FAIL_ARCHIVE"]));
    assert!(!guest.has("container"));
    assert!(!guest.has("state/created"));
    let calls = guest.api_calls();
    assert_eq!(
        calls.last().map(String::as_str),
        Some("DELETE /containers/srl1-abc12345?force=true")
    );
    assert!(!calls.iter().any(|call| call.ends_with("/start")));
    let reports = guest.reports();
    assert_eq!(reports.len(), 1);
    assert!(
        reports[0].starts_with("failed: ") && reports[0].ends_with("no space left on device"),
        "{reports:?}"
    );
}

#[test]
fn sandbox_058_a_container_a_first_boot_left_without_its_volumes_is_created_again_with_them() {
    let guest = FakeGuest::new();
    fs::write(guest.path("container"), "").unwrap();
    assert!(guest.boot(&[]));
    assert_eq!(
        guest.api_calls(),
        [
            "DELETE /containers/srl1-abc12345?force=true",
            "POST /containers/create?name=srl1-abc12345",
            "PUT /containers/srl1-abc12345/archive?path=/",
            "POST /containers/srl1-abc12345/start",
        ]
    );
    assert_eq!(
        fs::read_to_string(guest.path("created-from.json")).unwrap(),
        fs::read_to_string(guest.path("container.json")).unwrap()
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
fn sandbox_095_a_writable_volumes_empty_folders_are_copied_into_the_container_with_its_files() {
    let guest = FakeGuest::new();
    let certs = guest.dir.path().join("certs");
    fs::create_dir_all(certs.join("sub")).unwrap();
    fs::create_dir(certs.join("empty")).unwrap();
    fs::write(certs.join("sub/ca.crt"), "CA").unwrap();
    let writable = fs::read_to_string(guest.path("writable")).unwrap();
    fs::write(
        guest.path("writable"),
        format!("{writable}{} /etc/certs\n", certs.display()),
    )
    .unwrap();
    assert!(guest.boot(&[]));
    let listing = Command::new("tar")
        .arg("-tf")
        .arg(guest.path("copied.tar"))
        .output()
        .unwrap();
    let mut entries: Vec<String> = String::from_utf8(listing.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        [
            "./etc/certs/empty/",
            "./etc/certs/sub/ca.crt",
            "./etc/opt/srlinux/config.json",
        ]
    );
}

#[test]
fn sandbox_059_a_boot_after_the_container_was_made_only_starts_it() {
    let guest = FakeGuest::new();
    assert!(guest.boot(&[]));
    assert!(guest.boot(&[]));
    assert_eq!(guest.api_calls(), ["POST /containers/srl1-abc12345/start"]);
    assert_eq!(guest.reports(), ["started", "started"]);
}

#[test]
fn sandbox_081_a_create_docker_refuses_reports_dockers_message_and_copies_and_starts_nothing() {
    let guest = FakeGuest::new();
    assert!(!guest.boot(&["FAIL_CREATE"]));
    assert!(!guest.has("container"));
    assert!(!guest.has("state/created"));
    assert_eq!(
        guest.api_calls(),
        [
            "DELETE /containers/srl1-abc12345?force=true",
            "POST /containers/create?name=srl1-abc12345",
        ]
    );
    let reports = guest.reports();
    assert_eq!(reports.len(), 1);
    assert!(
        reports[0].starts_with("failed: ") && reports[0].ends_with("No such image: srl:26"),
        "{reports:?}"
    );
}

#[test]
fn sandbox_082_a_boot_whose_container_docker_already_restarted_reports_it_started() {
    let guest = FakeGuest::new();
    assert!(guest.boot(&[]));
    assert!(guest.boot(&["RUNNING"]));
    assert_eq!(guest.reports(), ["started", "started"]);
}

#[test]
fn sandbox_083_the_guest_calls_the_engine_api_version_it_is_given_on_the_vms_docker_socket() {
    let guest = FakeGuest::new();
    assert!(guest.boot(&[]));
    let prefix = format!("http://docker/v{API_VERSION}/containers/");
    let requests = guest.requests();
    assert_eq!(requests.len(), 4);
    assert!(
        requests
            .iter()
            .all(|request| request.split_once(' ').unwrap().1.starts_with(&prefix)),
        "{requests:?}"
    );
    assert_eq!(guest.lines("sockets"), vec![guest.path("docker.sock"); 4]);
    assert!(
        NodeScript::guest(API_VERSION)
            .render()
            .unwrap()
            .contains(r#"--unix-socket "/run/docker.sock""#)
    );
}
