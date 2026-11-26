//! The Ignition config a node VM boots with the first time: Flatcar with updates and reboots
//! off, no login, Docker's data on the VM's Docker store disk, Docker's API on vsock behind
//! mutual TLS, the NICs named for the network OS of the node's container, which runs in the
//! VM's network namespace, and the network OS CLI on the serial console. The VM that makes a
//! Docker store boots with the Docker part alone.

use std::fmt::Write as _;

use anyhow::Result;

use shared::util::base64_encode;
use template::{
    FileMode, IgnitionConfig, IgnitionFile, IgnitionFileContents, IgnitionFileSystem, IgnitionUnit,
};

use super::NODE_VM_DOCKER_PORT;

/// Where dockerd's TLS files go in the VM.
const DOCKER_TLS_DIR: &str = "/etc/docker/tls";
/// The local port dockerd's TLS listener takes; the vsock proxy hands connections to it.
const DOCKER_TLS_LOCAL_PORT: u16 = 2376;
/// containerd's config in the VM, and where it goes.
const CONTAINERD_CONFIG: &str = include_str!("containerd.toml");
const CONTAINERD_CONFIG_PATH: &str = "/etc/containerd/config.toml";
/// The script that names the NICs and makes the interfaces on no link, before the container
/// starts.
const LINKS_SCRIPT: &str = include_str!("links.sh");
/// The script that runs the network OS CLI on the serial console.
const CONSOLE_SCRIPT: &str = include_str!("console.sh");
/// The label of the filesystem on a node VM's Docker store disk.
const DOCKER_STORE_LABEL: &str = "sherpa-docker";
/// The Docker store disk, the VM's second disk.
const DOCKER_STORE_DEVICE: &str = "/dev/vdb";
/// Units a node VM never runs: updates and their reboots, SSH logins, the serial login, the
/// image's cloud metadata units, which wait for a network the VM does not have, its network
/// manager and its sockets, as the network OS owns every interface, and the first boot's
/// rebuilds of the library cache and the hardware database, which hold up Docker: the image
/// ships the library cache, and udev needs no hardware database for the VM's devices.
const MASKED_UNITS: [&str; 18] = [
    "update-engine.service",
    "locksmithd.service",
    "sshd.socket",
    "sshd-vsock.socket",
    "sshd.service",
    "serial-getty@ttyS0.service",
    "coreos-metadata-sshkeys@core.service",
    "sshkeys.service",
    "flatcar-openstack-hostname.service",
    "coreos-metadata.service",
    "systemd-networkd-wait-online.service",
    "systemd-networkd.service",
    "systemd-networkd.socket",
    "systemd-networkd-varlink.socket",
    "systemd-networkd-varlink-metrics.socket",
    "systemd-networkd-resolve-hook.socket",
    "ldconfig.service",
    "systemd-hwdb-update.service",
];

/// The TLS files of a VM's Docker API.
pub(super) struct DockerTls<'a> {
    pub(super) ca: &'a str,
    pub(super) server_cert: &'a str,
    pub(super) server_key: &'a str,
}

/// What a node VM's first boot sets up.
pub(super) struct GuestConfig<'a> {
    pub(super) hostname: &'a str,
    pub(super) container: &'a str,
    pub(super) interfaces: &'a [(String, String)],
    pub(super) placeholders: &'a [(String, String)],
    pub(super) tls: DockerTls<'a>,
    pub(super) console_command: &'a [&'a str],
}

/// A root-owned file at `path` with `mode` and `contents`.
fn file(path: &str, mode: u32, contents: &str) -> IgnitionFile {
    IgnitionFile {
        path: path.to_owned(),
        mode: FileMode::new(mode),
        overwrite: Some(true),
        contents: Some(IgnitionFileContents::new(&format!(
            "data:;base64,{}",
            base64_encode(contents)
        ))),
        ..Default::default()
    }
}

/// A unit `name` with `contents`, enabled when `enabled`.
fn unit(name: &str, enabled: bool, contents: String) -> IgnitionUnit {
    IgnitionUnit {
        name: name.to_owned(),
        enabled: Some(enabled),
        contents: Some(contents),
        ..Default::default()
    }
}

/// The guest's interfaces, its NICs or the veths made for interfaces on no link, one
/// `mac name` per line, the MAC in the lowercase form the guest's kernel reports.
fn interfaces_file(interfaces: &[(String, String)]) -> String {
    let mut file = String::new();
    for (mac, name) in interfaces {
        let _ = writeln!(file, "{} {name}", mac.to_ascii_lowercase());
    }
    file
}

/// The files of Docker on its store disk with its API behind TLS, and of updates off. Docker
/// sets up no iptables rules: the node's container has no default bridge and publishes no
/// ports, and the rules take Docker's start time. containerd runs without the CRI plugins,
/// whose start takes its time too.
fn docker_files(tls: &DockerTls<'_>) -> Vec<IgnitionFile> {
    let docker_options = format!(
        "[Service]\nEnvironment=\"DOCKER_OPTS=--host=tcp://127.0.0.1:{DOCKER_TLS_LOCAL_PORT} \
         --tlsverify --tlscacert={DOCKER_TLS_DIR}/ca.pem --tlscert={DOCKER_TLS_DIR}/server.pem \
         --tlskey={DOCKER_TLS_DIR}/server-key.pem --bridge=none --iptables=false \
         --ip6tables=false\"\n"
    );
    vec![
        file(
            "/etc/flatcar/update.conf",
            0o644,
            "SERVER=disabled\nREBOOT_STRATEGY=off\n",
        ),
        file(&format!("{DOCKER_TLS_DIR}/ca.pem"), 0o644, tls.ca),
        file(
            &format!("{DOCKER_TLS_DIR}/server.pem"),
            0o644,
            tls.server_cert,
        ),
        file(
            &format!("{DOCKER_TLS_DIR}/server-key.pem"),
            0o600,
            tls.server_key,
        ),
        file(
            "/etc/systemd/system/docker.service.d/10-sherpa.conf",
            0o644,
            &docker_options,
        ),
        file(CONTAINERD_CONFIG_PATH, 0o644, CONTAINERD_CONFIG),
        file(
            "/etc/systemd/system/containerd.service.d/10-sherpa.conf",
            0o644,
            &format!("[Service]\nEnvironment=CONTAINERD_CONFIG={CONTAINERD_CONFIG_PATH}\n"),
        ),
    ]
}

/// The files of the guest's first boot.
fn files(config: &GuestConfig<'_>) -> Vec<IgnitionFile> {
    let mut files = docker_files(&config.tls);
    files.extend([
        file("/etc/hostname", 0o644, &format!("{}\n", config.hostname)),
        file(
            "/etc/sherpa/container",
            0o644,
            &format!("{}\n", config.container),
        ),
        file(
            "/etc/sherpa/interfaces",
            0o644,
            &interfaces_file(config.interfaces),
        ),
        file(
            "/etc/sherpa/placeholders",
            0o644,
            &interfaces_file(config.placeholders),
        ),
        file("/opt/sherpa/links", 0o755, LINKS_SCRIPT),
        file("/opt/sherpa/console", 0o755, CONSOLE_SCRIPT),
    ]);
    files
}

/// The units of Docker on its store disk with its API on vsock, with updates, logins and
/// metadata off.
fn docker_units() -> Vec<IgnitionUnit> {
    let mut units: Vec<IgnitionUnit> = MASKED_UNITS
        .iter()
        .map(|name| IgnitionUnit {
            name: (*name).to_owned(),
            mask: Some(true),
            ..Default::default()
        })
        .collect();
    units.push(IgnitionUnit {
        name: "docker.service".to_owned(),
        enabled: Some(true),
        ..Default::default()
    });
    units.push(unit(
        "docker-vsock.socket",
        true,
        format!(
            "[Unit]\nDescription=Docker API on vsock\n\n[Socket]\n\
             ListenStream=vsock::{NODE_VM_DOCKER_PORT}\n\n[Install]\nWantedBy=sockets.target\n"
        ),
    ));
    units.push(unit(
        "docker-vsock.service",
        false,
        format!(
            "[Unit]\nDescription=Docker API on vsock\nRequires=docker.service \
             docker-vsock.socket\nAfter=docker.service docker-vsock.socket\n\n[Service]\n\
             ExecStart=/usr/lib/systemd/systemd-socket-proxyd 127.0.0.1:{DOCKER_TLS_LOCAL_PORT}\n"
        ),
    ));
    // Not lazily zeroing the inode tables keeps every node's writes off the shared store.
    units.push(unit(
        "var-lib-docker.mount",
        true,
        format!(
            "[Unit]\nDescription=Docker store\nBefore=containerd.service docker.service\n\n\
             [Mount]\nWhat=/dev/disk/by-label/{DOCKER_STORE_LABEL}\nWhere=/var/lib/docker\n\
             Type=ext4\nOptions=noinit_itable\n\n[Install]\n\
             RequiredBy=containerd.service docker.service\n"
        ),
    ));
    units
}

/// The units of the guest's first boot.
fn units(config: &GuestConfig<'_>) -> Vec<IgnitionUnit> {
    let mut units = docker_units();
    units.push(unit(
        "sherpa-links.service",
        true,
        "[Unit]\nDescription=Name the NICs for the network OS\nBefore=docker.service\n\n\
         [Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=/opt/sherpa/links\n\n\
         [Install]\nWantedBy=docker.service\n"
            .to_owned(),
    ));
    units.push(unit(
        "sherpa-console.service",
        true,
        format!(
            "[Unit]\nDescription=Network OS console\nRequires=docker.service\n\
             After=docker.service\nStartLimitIntervalSec=0\n\n[Service]\n\
             ExecStart=/opt/sherpa/console {}\nStandardInput=tty\nStandardOutput=tty\n\
             TTYPath=/dev/ttyS0\nTTYReset=yes\nTTYVHangup=yes\nRestart=always\nRestartSec=1\n\n\
             [Install]\nWantedBy=multi-user.target\n",
            config.console_command.join(" ")
        ),
    ));
    units
}

/// The Ignition config of a node VM's first boot, as JSON.
///
/// # Errors
///
/// Fails when the config cannot be serialized.
pub(super) fn ignition_json(config: &GuestConfig<'_>) -> Result<String> {
    IgnitionConfig::new(vec![], files(config), vec![], units(config), vec![], vec![])
        .to_json_pretty()
}

/// The Ignition config of the VM that makes a Docker store, as JSON: Docker as in a node VM,
/// on its empty second disk, which it formats.
///
/// # Errors
///
/// Fails when the config cannot be serialized.
pub(super) fn store_ignition_json(tls: &DockerTls<'_>) -> Result<String> {
    IgnitionConfig::new(
        vec![],
        docker_files(tls),
        vec![],
        docker_units(),
        vec![],
        vec![IgnitionFileSystem {
            device: DOCKER_STORE_DEVICE.to_owned(),
            format: "ext4".to_owned(),
            wipe_filesystem: false,
            label: DOCKER_STORE_LABEL.to_owned(),
        }],
    )
    .to_json_pretty()
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::Value;
    use shared::util::base64_decode;

    fn config() -> Value {
        let interfaces = [
            ("52:54:00:AB:00:01".to_owned(), "eth0".to_owned()),
            ("52:54:00:AB:00:02".to_owned(), "eth1".to_owned()),
        ];
        let json = ignition_json(&GuestConfig {
            hostname: "ceos1",
            container: "ceos1-abc12345",
            interfaces: &interfaces,
            placeholders: &[
                ("52:54:00:AB:00:03".to_owned(), "eth2".to_owned()),
                ("52:54:00:AB:00:04".to_owned(), "eth3".to_owned()),
            ],
            tls: DockerTls {
                ca: "CA",
                server_cert: "CERT",
                server_key: "KEY",
            },
            console_command: &["Cli"],
        })
        .unwrap();
        serde_json::from_str(&json).unwrap()
    }

    fn file_text(config: &Value, path: &str) -> String {
        let source = config["storage"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == path)
            .unwrap_or_else(|| panic!("no {path}"))["contents"]["source"]
            .as_str()
            .unwrap()
            .to_owned();
        let encoded = source.strip_prefix("data:;base64,").unwrap();
        base64_decode(encoded).unwrap()
    }

    fn unit_of<'a>(config: &'a Value, name: &str) -> &'a Value {
        config["systemd"]["units"]
            .as_array()
            .unwrap()
            .iter()
            .find(|unit| unit["name"] == name)
            .unwrap_or_else(|| panic!("no {name}"))
    }

    #[test]
    fn node_vm_008_updates_reboots_and_logins_are_off_in_the_node_vm() {
        let config = config();
        assert_eq!(
            file_text(&config, "/etc/flatcar/update.conf"),
            "SERVER=disabled\nREBOOT_STRATEGY=off\n"
        );
        for name in [
            "update-engine.service",
            "locksmithd.service",
            "sshd.socket",
            "sshd-vsock.socket",
            "serial-getty@ttyS0.service",
        ] {
            assert_eq!(unit_of(&config, name)["mask"], true, "{name}");
        }
        assert_eq!(config["passwd"]["users"], serde_json::json!([]));
    }

    #[test]
    fn node_vm_040_docker_in_the_node_vm_sets_up_no_iptables_rules() {
        let options = file_text(
            &config(),
            "/etc/systemd/system/docker.service.d/10-sherpa.conf",
        );
        assert!(options.contains(" --iptables=false --ip6tables=false"));
    }

    #[test]
    fn node_vm_041_containerd_in_the_node_vm_runs_without_the_cri_plugins() {
        let config = config();
        assert_eq!(
            file_text(
                &config,
                "/etc/systemd/system/containerd.service.d/10-sherpa.conf"
            ),
            "[Service]\nEnvironment=CONTAINERD_CONFIG=/etc/containerd/config.toml\n"
        );
        let containerd: toml::Value =
            toml::from_str(&file_text(&config, "/etc/containerd/config.toml")).unwrap();
        let disabled = containerd["disabled_plugins"].as_array().unwrap();
        assert!(disabled.contains(&toml::Value::from("io.containerd.grpc.v1.cri")));
        assert_eq!(
            containerd["grpc"]["address"].as_str(),
            Some("/run/containerd/containerd.sock")
        );
    }

    #[test]
    fn node_vm_039_the_first_boot_does_not_rebuild_the_library_cache_or_hardware_database() {
        let config = config();
        for name in ["ldconfig.service", "systemd-hwdb-update.service"] {
            assert_eq!(unit_of(&config, name)["mask"], true, "{name}");
        }
    }

    #[test]
    fn node_vm_009_docker_listens_only_on_loopback_with_client_certificates_behind_vsock() {
        let config = config();
        let options = file_text(
            &config,
            "/etc/systemd/system/docker.service.d/10-sherpa.conf",
        );
        assert!(options.contains("--host=tcp://127.0.0.1:2376 --tlsverify"));
        assert!(options.contains("--tlscacert=/etc/docker/tls/ca.pem"));
        assert!(
            unit_of(&config, "docker-vsock.socket")["contents"]
                .as_str()
                .unwrap()
                .contains("ListenStream=vsock::2376")
        );
        let key = config["storage"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == "/etc/docker/tls/server-key.pem")
            .unwrap();
        assert_eq!(key["mode"], 0o600);
    }

    #[test]
    fn node_vm_010_the_nics_are_named_by_their_lowercase_mac_before_docker_and_nothing_manages_them()
     {
        let config = config();
        assert_eq!(
            file_text(&config, "/etc/sherpa/interfaces"),
            "52:54:00:ab:00:01 eth0\n52:54:00:ab:00:02 eth1\n"
        );
        let links = unit_of(&config, "sherpa-links.service")["contents"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(links.contains("Before=docker.service\n"));
        assert!(links.contains("Type=oneshot\n"));
        assert!(links.contains("WantedBy=docker.service\n"));
        assert!(file_text(&config, "/opt/sherpa/links").contains("done < /etc/sherpa/interfaces"));
        for name in ["systemd-networkd.service", "systemd-networkd.socket"] {
            assert_eq!(unit_of(&config, name)["mask"], true, "{name}");
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
    fn node_vm_035_interfaces_on_no_link_are_veths_with_a_fixed_mac_and_a_peer_down_elsewhere() {
        let config = config();
        assert_eq!(
            file_text(&config, "/etc/sherpa/placeholders"),
            "52:54:00:ab:00:03 eth2\n52:54:00:ab:00:04 eth3\n"
        );
        let links = file_text(&config, "/opt/sherpa/links");
        assert!(links.contains(r#"ip link add "$want" address "$mac" mtu 9600 type veth"#));
        assert!(links.contains(r#"peer name "unw$index" mtu 9600 netns "$UNWIRED_NETNS""#));
        assert!(links.contains("done < /etc/sherpa/placeholders"));
    }

    #[test]
    fn node_vm_036_docker_keeps_its_data_on_the_store_disk_which_the_store_vm_formats() {
        let mount = unit_of(&config(), "var-lib-docker.mount")["contents"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(mount.contains("What=/dev/disk/by-label/sherpa-docker\nWhere=/var/lib/docker\n"));
        assert!(mount.contains("RequiredBy=containerd.service docker.service"));
        let node: Value = config();
        assert_eq!(node["storage"]["filesystems"], serde_json::json!([]));
        let store: Value = serde_json::from_str(
            &store_ignition_json(&DockerTls {
                ca: "CA",
                server_cert: "CERT",
                server_key: "KEY",
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(store["storage"]["filesystems"][0]["device"], "/dev/vdb");
        assert_eq!(store["storage"]["filesystems"][0]["label"], "sherpa-docker");
        assert_eq!(unit_of(&store, "var-lib-docker.mount")["enabled"], true);
        assert!(
            store["systemd"]["units"]
                .as_array()
                .unwrap()
                .iter()
                .all(|unit| unit["name"] != "sherpa-links.service")
        );
    }

    #[test]
    fn node_vm_011_the_serial_console_runs_the_network_os_cli() {
        let config = config();
        let console = unit_of(&config, "sherpa-console.service")["contents"]
            .as_str()
            .unwrap();
        assert!(console.contains("ExecStart=/opt/sherpa/console Cli\n"));
        assert!(console.contains("TTYPath=/dev/ttyS0"));
    }
}
