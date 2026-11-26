//! The Ignition configs of node VMs, the Flatcar VMs that each run one container node, and of
//! the VMs that make their Docker stores. See `docs/architecture/NODE_VM.md`.

use std::fmt::Write as _;
use std::net::IpAddr;

use anyhow::Result;

use crate::ignition::{IgnitionConfig, IgnitionFile, IgnitionFileSystem, IgnitionUnit};

/// The volume label Flatcar's Ignition looks for on a config drive.
pub const NODE_VM_CONFIG_DRIVE_LABEL: &str = "config-2";
/// Where Ignition reads its config on a config drive.
pub const NODE_VM_CONFIG_DRIVE_USER_DATA: &str = "openstack/latest/user_data";
/// Where a node VM mounts its config drive.
pub const NODE_VM_CONFIG_DRIVE_MOUNT: &str = "/media/config";
/// The serial of the disk that holds the image archive a Docker store VM loads.
pub const NODE_VM_IMAGE_ARCHIVE_SERIAL: &str = "sherpa-image";
/// The name of the virtio-serial port a node VM reports its status on.
pub const NODE_VM_STATUS_CHANNEL: &str = "org.sherpa.status";
/// The interface name of a node VM's management NIC.
pub const NODE_VM_MANAGEMENT_INTERFACE: &str = "eth0";

const LINKS_SCRIPT: &str = include_str!("../templates/node_vm/links.sh");
const NODE_SCRIPT: &str = include_str!("../templates/node_vm/node.sh");
const STORE_SCRIPT: &str = include_str!("../templates/node_vm/store.sh");
const REPORT_SCRIPT: &str = include_str!("../templates/node_vm/report.sh");
const CONSOLE_SCRIPT: &str = include_str!("../templates/node_vm/console.sh");
const CONTAINERD_CONFIG: &str = include_str!("../templates/node_vm/containerd.toml");
const CONTAINERD_CONFIG_PATH: &str = "/etc/containerd/config.toml";
const DOCKER_STORE_LABEL: &str = "sherpa-docker";
const DOCKER_STORE_DEVICE: &str = "/dev/vdb";
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

/// A network interface of a node VM: a NIC found by its MAC, or a veth made with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestInterface {
    pub mac: String,
    pub name: String,
}

/// An address of a node VM's management NIC with its prefix length, and the gateway of the
/// default route of its address family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestAddress {
    pub address: IpAddr,
    pub prefix_length: u8,
    pub gateway: IpAddr,
}

/// A writable volume of a node VM's container: its file or folder on the config drive, and its
/// path in the container, which it is copied to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritableVolume {
    pub source: String,
    pub destination: String,
}

/// What a node VM's first boot sets up.
#[derive(Debug)]
pub struct NodeVmGuest<'a> {
    pub hostname: &'a str,
    pub container: &'a str,
    pub interfaces: &'a [GuestInterface],
    pub placeholders: &'a [GuestInterface],
    pub management: &'a [GuestAddress],
    pub console_command: &'a [&'a str],
    pub create_args: &'a [String],
    pub writable: &'a [WritableVolume],
}

impl IgnitionFile {
    /// The Docker options of a node VM: no default bridge and no iptables rules, as the node's
    /// container runs in the VM's network namespace and publishes no ports.
    #[must_use]
    pub fn node_vm_docker_options() -> Self {
        Self::root(
            "/etc/systemd/system/docker.service.d/10-sherpa.conf",
            0o644,
            r#"[Service]
Environment="DOCKER_OPTS=--bridge=none --iptables=false --ip6tables=false"
"#,
        )
    }

    /// containerd's config in a node VM: Flatcar's without the CRI plugins and NRI.
    #[must_use]
    pub fn node_vm_containerd_config() -> Self {
        Self::root(CONTAINERD_CONFIG_PATH, 0o644, CONTAINERD_CONFIG)
    }

    /// The drop-in that points containerd at its node VM config.
    #[must_use]
    pub fn node_vm_containerd_options() -> Self {
        Self::root(
            "/etc/systemd/system/containerd.service.d/10-sherpa.conf",
            0o644,
            &format!("[Service]\nEnvironment=CONTAINERD_CONFIG={CONTAINERD_CONFIG_PATH}\n"),
        )
    }
}

impl IgnitionUnit {
    /// Unit `name`, masked.
    #[must_use]
    pub fn masked(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            mask: Some(true),
            ..Default::default()
        }
    }

    /// Unit `name` with `contents`, enabled.
    #[must_use]
    pub fn enabled(name: &str, contents: &str) -> Self {
        Self {
            name: name.to_owned(),
            enabled: Some(true),
            contents: Some(contents.to_owned()),
            ..Default::default()
        }
    }

    /// Mount the Docker store disk at `/var/lib/docker` before containerd and Docker start,
    /// without zeroing its inode tables.
    #[must_use]
    pub fn mount_docker_store() -> Self {
        Self::enabled(
            "var-lib-docker.mount",
            r"[Unit]
Description=Docker store
Before=containerd.service docker.service

[Mount]
What=/dev/disk/by-label/sherpa-docker
Where=/var/lib/docker
Type=ext4
Options=noinit_itable

[Install]
RequiredBy=containerd.service docker.service
",
        )
    }

    /// Mount the config drive read-only at `/media/config`.
    #[must_use]
    pub fn mount_config_drive() -> Self {
        Self::enabled(
            "media-config.mount",
            r"[Unit]
Description=Node config drive

[Mount]
What=/dev/disk/by-label/config-2
Where=/media/config
Type=iso9660
Options=ro

[Install]
WantedBy=local-fs.target
",
        )
    }

    /// Name and address the NICs for the network OS, once per boot, before Docker starts.
    #[must_use]
    pub fn node_vm_links() -> Self {
        Self::enabled(
            "sherpa-links.service",
            r"[Unit]
Description=Name and address the NICs for the network OS
Before=docker.service

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/opt/sherpa/links

[Install]
WantedBy=docker.service
",
        )
    }

    /// Start the node's container once Docker runs and the NICs are named.
    #[must_use]
    pub fn node_vm_container() -> Self {
        Self::enabled(
            "sherpa-node.service",
            r"[Unit]
Description=The node's container
Requires=docker.service sherpa-links.service
After=docker.service sherpa-links.service
RequiresMountsFor=/media/config

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/opt/sherpa/node

[Install]
WantedBy=multi-user.target
",
        )
    }

    /// Run the network OS CLI on the serial console.
    #[must_use]
    pub fn node_vm_console() -> Self {
        Self::enabled(
            "sherpa-console.service",
            r"[Unit]
Description=Network OS console
Requires=docker.service
After=docker.service
StartLimitIntervalSec=0

[Service]
ExecStart=/opt/sherpa/console
StandardInput=tty
StandardOutput=tty
TTYPath=/dev/ttyS0
TTYReset=yes
TTYVHangup=yes
Restart=always
RestartSec=1

[Install]
WantedBy=multi-user.target
",
        )
    }

    /// Load the node image into the Docker store once Docker runs, then power off.
    #[must_use]
    pub fn node_vm_store() -> Self {
        Self::enabled(
            "sherpa-store.service",
            r"[Unit]
Description=Load the node image into the Docker store
Requires=docker.service
After=docker.service

[Service]
Type=oneshot
ExecStart=/opt/sherpa/store

[Install]
WantedBy=multi-user.target
",
        )
    }
}

impl NodeVmGuest<'_> {
    /// The Ignition config of the node VM's first boot, as JSON.
    ///
    /// # Errors
    ///
    /// Fails when the config cannot be serialized.
    pub fn ignition_json(&self) -> Result<String> {
        let mut files = docker_files();
        files.extend([
            IgnitionFile::hostname(self.hostname),
            IgnitionFile::root(
                "/etc/sherpa/container",
                0o644,
                &format!("{}\n", self.container),
            ),
            IgnitionFile::root(
                "/etc/sherpa/interfaces",
                0o644,
                &interfaces_file(self.interfaces),
            ),
            IgnitionFile::root(
                "/etc/sherpa/placeholders",
                0o644,
                &interfaces_file(self.placeholders),
            ),
            IgnitionFile::root(
                "/etc/sherpa/management",
                0o644,
                &management_file(self.management),
            ),
            IgnitionFile::root(
                "/etc/sherpa/console",
                0o644,
                &lines(self.console_command.iter()),
            ),
            IgnitionFile::root(
                "/etc/sherpa/create",
                0o644,
                &nul_separated(self.create_args),
            ),
            IgnitionFile::root("/etc/sherpa/writable", 0o644, &writable_file(self.writable)),
            IgnitionFile::root("/opt/sherpa/links", 0o755, LINKS_SCRIPT),
            IgnitionFile::root("/opt/sherpa/node", 0o755, NODE_SCRIPT),
            IgnitionFile::root("/opt/sherpa/console", 0o755, CONSOLE_SCRIPT),
        ]);
        let mut units = docker_units();
        units.extend([
            IgnitionUnit::mount_config_drive(),
            IgnitionUnit::node_vm_links(),
            IgnitionUnit::node_vm_container(),
            IgnitionUnit::node_vm_console(),
        ]);
        IgnitionConfig::new(vec![], files, vec![], units, vec![], vec![]).to_json_pretty()
    }
}

/// The Ignition config of the VM that makes a Docker store, as JSON: Docker as in a node VM, on
/// its empty second disk, which it formats, and the unit that loads the image archive into it.
///
/// # Errors
///
/// Fails when the config cannot be serialized.
pub fn store_vm_ignition_json() -> Result<String> {
    let mut files = docker_files();
    files.push(IgnitionFile::root("/opt/sherpa/store", 0o755, STORE_SCRIPT));
    let mut units = docker_units();
    units.push(IgnitionUnit::node_vm_store());
    IgnitionConfig::new(
        vec![],
        files,
        vec![],
        units,
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

/// The files of Docker and containerd, of updates off, and of the status report, in every
/// Flatcar VM Sherpa runs for a node.
fn docker_files() -> Vec<IgnitionFile> {
    vec![
        IgnitionFile::disable_updates(),
        IgnitionFile::node_vm_docker_options(),
        IgnitionFile::node_vm_containerd_config(),
        IgnitionFile::node_vm_containerd_options(),
        IgnitionFile::root("/opt/sherpa/report", 0o755, REPORT_SCRIPT),
    ]
}

/// The units of Docker on its store disk, with updates, logins, metadata, networking and the
/// first boot's cache rebuilds off.
fn docker_units() -> Vec<IgnitionUnit> {
    let mut units: Vec<IgnitionUnit> = MASKED_UNITS
        .iter()
        .map(|name| IgnitionUnit::masked(name))
        .collect();
    units.push(IgnitionUnit {
        name: "docker.service".to_owned(),
        enabled: Some(true),
        ..Default::default()
    });
    units.push(IgnitionUnit::mount_docker_store());
    units
}

/// One `mac name` line per interface, the MAC in the lowercase form the guest's kernel reports.
fn interfaces_file(interfaces: &[GuestInterface]) -> String {
    let mut file = String::new();
    for GuestInterface { mac, name } in interfaces {
        let _ = writeln!(file, "{} {name}", mac.to_ascii_lowercase());
    }
    file
}

/// One `interface address/prefix gateway` line per management address.
fn management_file(addresses: &[GuestAddress]) -> String {
    let mut file = String::new();
    for GuestAddress {
        address,
        prefix_length,
        gateway,
    } in addresses
    {
        let _ = writeln!(
            file,
            "{NODE_VM_MANAGEMENT_INTERFACE} {address}/{prefix_length} {gateway}"
        );
    }
    file
}

/// One `source destination` line per writable volume.
fn writable_file(volumes: &[WritableVolume]) -> String {
    lines(
        volumes
            .iter()
            .map(|volume| format!("{} {}", volume.source, volume.destination)),
    )
}

/// Each of `items` on a line of its own.
fn lines(items: impl Iterator<Item = impl AsRef<str>>) -> String {
    let mut file = String::new();
    for item in items {
        file.push_str(item.as_ref());
        file.push('\n');
    }
    file
}

/// Each of `args` followed by a NUL byte, which no argument holds.
fn nul_separated(args: &[String]) -> String {
    let mut file = String::new();
    for arg in args {
        file.push_str(arg);
        file.push('\0');
    }
    file
}
