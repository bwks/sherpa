//! The Ignition configs of node VMs, the Flatcar VMs that each run one container node, and of
//! the VMs that make their Docker stores. See `docs/architecture/NODE_VM.md`.

use std::net::IpAddr;

use anyhow::Result;
use askama::Template;

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
/// The terminal of a node VM's serial console.
pub const NODE_VM_CONSOLE_TTY: &str = "ttyS0";

const CONTAINERD_CONFIG: &str = include_str!("../templates/node_vm/containerd.toml");
const CONTAINERD_CONFIG_PATH: &str = "/etc/containerd/config.toml";
const DOCKER_DROP_IN_PATH: &str = "/etc/systemd/system/docker.service.d/10-sherpa.conf";
const CONTAINERD_DROP_IN_PATH: &str = "/etc/systemd/system/containerd.service.d/10-sherpa.conf";
const DOCKER_STORE_LABEL: &str = "sherpa-docker";
const DOCKER_STORE_DEVICE: &str = "/dev/vdb";
const DOCKER_DATA_DIR: &str = "/var/lib/docker";
const CONTAINER_FILE: &str = "/etc/sherpa/container";
const INTERFACES_FILE: &str = "/etc/sherpa/interfaces";
const PLACEHOLDERS_FILE: &str = "/etc/sherpa/placeholders";
const MANAGEMENT_FILE: &str = "/etc/sherpa/management";
const CONSOLE_COMMAND_FILE: &str = "/etc/sherpa/console";
const CREATE_FILE: &str = "/etc/sherpa/create";
const WRITABLE_FILE: &str = "/etc/sherpa/writable";
const LINKS_SCRIPT_PATH: &str = "/opt/sherpa/links";
const NODE_SCRIPT_PATH: &str = "/opt/sherpa/node";
const CONSOLE_SCRIPT_PATH: &str = "/opt/sherpa/console";
const REPORT_SCRIPT_PATH: &str = "/opt/sherpa/report";
const STORE_SCRIPT_PATH: &str = "/opt/sherpa/store";
const LINKS_UNIT: &str = "sherpa-links.service";
const NODE_UNIT: &str = "sherpa-node.service";
const CONSOLE_UNIT: &str = "sherpa-console.service";
const STORE_UNIT: &str = "sherpa-store.service";
const NUL: char = '\0';
const FILE_MODE: u32 = 0o644;
const SCRIPT_MODE: u32 = 0o755;
const MASKED_UNITS: [&str; 17] = [
    "update-engine.service",
    "locksmithd.service",
    "sshd.socket",
    "sshd-vsock.socket",
    "sshd.service",
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

/// An address of a node VM's management NIC with its prefix length and its gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestAddress {
    pub address: IpAddr,
    pub prefix_length: u8,
    pub gateway: IpAddr,
}

/// A writable volume of a node VM's container: its source on the config drive and its path in
/// the container.
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

/// The script that creates the node's container on its first boot and starts it on every boot.
#[derive(Debug, Template)]
#[template(path = "node_vm/node.sh.jinja", ext = "txt")]
pub struct NodeScript<'a> {
    pub container: &'a str,
    pub create: &'a str,
    pub writable: &'a str,
    pub report: &'a str,
}

/// Docker's drop-in in a node VM.
#[derive(Template)]
#[template(path = "node_vm/docker.conf.jinja", ext = "txt")]
struct DockerOptions;

/// containerd's drop-in in a node VM.
#[derive(Template)]
#[template(path = "node_vm/containerd.conf.jinja", ext = "txt")]
struct ContainerdOptions;

/// The mount unit of the Docker store disk.
#[derive(Template)]
#[template(path = "node_vm/docker-store.mount.jinja", ext = "txt")]
struct DockerStoreMount;

/// The mount unit of the config drive.
#[derive(Template)]
#[template(path = "node_vm/config-drive.mount.jinja", ext = "txt")]
struct ConfigDriveMount;

/// The unit that names and addresses the NICs.
#[derive(Template)]
#[template(path = "node_vm/sherpa-links.service.jinja", ext = "txt")]
struct LinksUnit;

/// The unit that starts the node's container.
#[derive(Template)]
#[template(path = "node_vm/sherpa-node.service.jinja", ext = "txt")]
struct NodeUnit;

/// The unit of the network OS CLI on the serial console.
#[derive(Template)]
#[template(path = "node_vm/sherpa-console.service.jinja", ext = "txt")]
struct ConsoleUnit;

/// The unit that loads the image archive into a Docker store.
#[derive(Template)]
#[template(path = "node_vm/sherpa-store.service.jinja", ext = "txt")]
struct StoreUnit;

/// The script that names and addresses the NICs.
#[derive(Template)]
#[template(path = "node_vm/links.sh.jinja", ext = "txt")]
struct LinksScript;

/// The script of the serial console.
#[derive(Template)]
#[template(path = "node_vm/console.sh.jinja", ext = "txt")]
struct ConsoleScript;

/// The script that writes a status line to the status channel.
#[derive(Template)]
#[template(path = "node_vm/report.sh.jinja", ext = "txt")]
struct ReportScript;

/// The script that loads the image archive into a Docker store.
#[derive(Template)]
#[template(path = "node_vm/store.sh.jinja", ext = "txt")]
struct StoreScript;

/// The node's container name, for the scripts that run it.
#[derive(Template)]
#[template(path = "node_vm/container.jinja", ext = "txt")]
struct ContainerFile<'a> {
    name: &'a str,
}

/// The NICs or veths the guest names or makes, one `mac name` line each.
#[derive(Template)]
#[template(path = "node_vm/interfaces.jinja", ext = "txt")]
struct InterfacesFile<'a> {
    interfaces: &'a [GuestInterface],
}

/// The management addresses, one `interface address/prefix gateway` line each.
#[derive(Template)]
#[template(path = "node_vm/management.jinja", ext = "txt")]
struct ManagementFile<'a> {
    addresses: &'a [GuestAddress],
}

/// The writable volumes to copy into the container, one `source destination` line each.
#[derive(Template)]
#[template(path = "node_vm/writable.jinja", ext = "txt")]
struct WritableFile<'a> {
    volumes: &'a [WritableVolume],
}

/// The console's command, one argument per line.
#[derive(Template)]
#[template(path = "node_vm/console-command.jinja", ext = "txt")]
struct ConsoleCommandFile<'a> {
    command: &'a [&'a str],
}

/// The container's `docker create` arguments, each followed by a NUL byte, which no argument
/// holds.
#[derive(Template)]
#[template(path = "node_vm/create.jinja", ext = "txt")]
struct CreateFile<'a> {
    args: &'a [String],
}

impl NodeScript<'static> {
    /// The script as a node VM runs it, with the files sherpad gives the guest.
    #[must_use]
    pub fn guest() -> Self {
        Self {
            container: CONTAINER_FILE,
            create: CREATE_FILE,
            writable: WRITABLE_FILE,
            report: REPORT_SCRIPT_PATH,
        }
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
}

impl NodeVmGuest<'_> {
    /// The Ignition config of the node VM's first boot, as JSON.
    ///
    /// # Errors
    ///
    /// Fails when a file or unit cannot be rendered or the config cannot be serialized.
    pub fn ignition_json(&self) -> Result<String> {
        let mut files = docker_files()?;
        files.extend([
            IgnitionFile::hostname(self.hostname),
            IgnitionFile::root(
                CONTAINER_FILE,
                FILE_MODE,
                &ContainerFile {
                    name: self.container,
                }
                .render()?,
            ),
            IgnitionFile::root(
                INTERFACES_FILE,
                FILE_MODE,
                &InterfacesFile {
                    interfaces: self.interfaces,
                }
                .render()?,
            ),
            IgnitionFile::root(
                PLACEHOLDERS_FILE,
                FILE_MODE,
                &InterfacesFile {
                    interfaces: self.placeholders,
                }
                .render()?,
            ),
            IgnitionFile::root(
                MANAGEMENT_FILE,
                FILE_MODE,
                &ManagementFile {
                    addresses: self.management,
                }
                .render()?,
            ),
            IgnitionFile::root(
                CONSOLE_COMMAND_FILE,
                FILE_MODE,
                &ConsoleCommandFile {
                    command: self.console_command,
                }
                .render()?,
            ),
            IgnitionFile::root(
                CREATE_FILE,
                FILE_MODE,
                &CreateFile {
                    args: self.create_args,
                }
                .render()?,
            ),
            IgnitionFile::root(
                WRITABLE_FILE,
                FILE_MODE,
                &WritableFile {
                    volumes: self.writable,
                }
                .render()?,
            ),
            IgnitionFile::root(LINKS_SCRIPT_PATH, SCRIPT_MODE, &LinksScript.render()?),
            IgnitionFile::root(
                NODE_SCRIPT_PATH,
                SCRIPT_MODE,
                &NodeScript::guest().render()?,
            ),
            IgnitionFile::root(CONSOLE_SCRIPT_PATH, SCRIPT_MODE, &ConsoleScript.render()?),
        ]);
        let mut units = docker_units()?;
        units.extend([
            IgnitionUnit::enabled(
                &mount_unit(NODE_VM_CONFIG_DRIVE_MOUNT),
                &ConfigDriveMount.render()?,
            ),
            IgnitionUnit::enabled(LINKS_UNIT, &LinksUnit.render()?),
            IgnitionUnit::enabled(NODE_UNIT, &NodeUnit.render()?),
            IgnitionUnit::enabled(CONSOLE_UNIT, &ConsoleUnit.render()?),
        ]);
        IgnitionConfig::new(vec![], files, vec![], units, vec![], vec![]).to_json_pretty()
    }
}

/// The Ignition config of the VM that makes a Docker store, as JSON: Docker as in a node VM, on
/// its empty second disk, which it formats, and the unit that loads the image archive into it.
///
/// # Errors
///
/// Fails when a file or unit cannot be rendered or the config cannot be serialized.
pub fn store_vm_ignition_json() -> Result<String> {
    let mut files = docker_files()?;
    files.push(IgnitionFile::root(
        STORE_SCRIPT_PATH,
        SCRIPT_MODE,
        &StoreScript.render()?,
    ));
    let mut units = docker_units()?;
    units.push(IgnitionUnit::enabled(STORE_UNIT, &StoreUnit.render()?));
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
fn docker_files() -> Result<Vec<IgnitionFile>> {
    Ok(vec![
        IgnitionFile::disable_updates(),
        IgnitionFile::root(DOCKER_DROP_IN_PATH, FILE_MODE, &DockerOptions.render()?),
        IgnitionFile::root(CONTAINERD_CONFIG_PATH, FILE_MODE, CONTAINERD_CONFIG),
        IgnitionFile::root(
            CONTAINERD_DROP_IN_PATH,
            FILE_MODE,
            &ContainerdOptions.render()?,
        ),
        IgnitionFile::root(REPORT_SCRIPT_PATH, SCRIPT_MODE, &ReportScript.render()?),
    ])
}

/// The units of Docker on its store disk, with updates, logins, metadata, networking and the
/// first boot's cache rebuilds off.
fn docker_units() -> Result<Vec<IgnitionUnit>> {
    let mut units: Vec<IgnitionUnit> = MASKED_UNITS
        .iter()
        .map(|name| IgnitionUnit::masked(name))
        .collect();
    units.push(IgnitionUnit::masked(&format!(
        "serial-getty@{NODE_VM_CONSOLE_TTY}.service"
    )));
    units.push(IgnitionUnit {
        name: "docker.service".to_owned(),
        enabled: Some(true),
        ..Default::default()
    });
    units.push(IgnitionUnit::enabled(
        &mount_unit(DOCKER_DATA_DIR),
        &DockerStoreMount.render()?,
    ));
    Ok(units)
}

/// The name of the mount unit of absolute path `path`, which holds no character systemd
/// escapes but `/`.
fn mount_unit(path: &str) -> String {
    format!("{}.mount", path.trim_start_matches('/').replace('/', "-"))
}
