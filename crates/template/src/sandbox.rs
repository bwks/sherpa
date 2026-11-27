//! The Ignition configs of sandboxes, the Flatcar VMs that each run one container node, and of
//! the VMs that make their Docker stores. See `docs/architecture/SANDBOX.md`.

use std::net::IpAddr;

use anyhow::Result;
use askama::Template;

use crate::ignition::{IgnitionConfig, IgnitionFile, IgnitionFileSystem, IgnitionUnit};

/// The volume label Flatcar's Ignition looks for on a config drive.
pub const SANDBOX_CONFIG_DRIVE_LABEL: &str = "config-2";
/// Where Ignition reads its config on a config drive.
pub const SANDBOX_CONFIG_DRIVE_USER_DATA: &str = "openstack/latest/user_data";
/// Where a sandbox mounts its config drive.
pub const SANDBOX_CONFIG_DRIVE_MOUNT: &str = "/media/config";
/// The serial of the disk that holds the image archive a Docker store VM loads.
pub const SANDBOX_IMAGE_ARCHIVE_SERIAL: &str = "sherpa-image";
/// The name of the virtio-serial port a sandbox reports its status on.
pub const SANDBOX_STATUS_CHANNEL: &str = "org.sherpa.status";
/// The interface name of a sandbox's management NIC.
pub const SANDBOX_MANAGEMENT_INTERFACE: &str = "eth0";
/// The terminal of a sandbox's serial console.
pub const SANDBOX_CONSOLE_TTY: &str = "ttyS0";
/// The network namespace of the sandbox's own NIC, which the network OS does not see.
pub const SANDBOX_NETNS: &str = "sherpa-sandbox";
/// The name of the sandbox's own NIC in its network namespace.
pub const SANDBOX_NIC: &str = "sbx0";

const CONTAINERD_CONFIG: &str = include_str!("../templates/sandbox/containerd.toml");
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
const SANDBOX_NIC_FILE: &str = "/etc/sherpa/sandbox-nic";
const CONSOLE_COMMAND_FILE: &str = "/etc/sherpa/console";
const CREATE_FILE: &str = "/etc/sherpa/create";
const WRITABLE_FILE: &str = "/etc/sherpa/writable";
const CREATED_MARKER: &str = "/var/lib/sherpa/created";
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

/// A network interface of a sandbox: a NIC found by its MAC, or a veth made with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestInterface {
    pub mac: String,
    pub name: String,
}

/// An address of a sandbox's management NIC with its prefix length and its gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestAddress {
    pub address: IpAddr,
    pub prefix_length: u8,
    pub gateway: IpAddr,
}

/// The sandbox's own NIC on the lab's management network, found by its MAC, and its addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxOwnNic {
    pub mac: String,
    pub addresses: Vec<GuestAddress>,
}

/// A writable volume of a sandbox's container: its source on the config drive and its path in
/// the container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritableVolume {
    pub source: String,
    pub destination: String,
}

/// What a sandbox's first boot sets up.
#[derive(Debug)]
pub struct SandboxGuest<'a> {
    pub hostname: &'a str,
    pub container: &'a str,
    pub interfaces: &'a [GuestInterface],
    pub placeholders: &'a [GuestInterface],
    pub management: &'a [GuestAddress],
    pub own_nic: &'a SandboxOwnNic,
    pub console_command: &'a [&'a str],
    pub create_args: &'a [String],
    pub writable: &'a [WritableVolume],
}

/// The script that creates the node's container on its first boot and starts it on every boot.
#[derive(Debug, Template)]
#[template(path = "sandbox/node.sh.jinja", ext = "txt")]
pub struct NodeScript<'a> {
    pub container: &'a str,
    pub create: &'a str,
    pub writable: &'a str,
    pub report: &'a str,
    pub created: &'a str,
}

/// Docker's drop-in in a sandbox.
#[derive(Template)]
#[template(path = "sandbox/docker.conf.jinja", ext = "txt")]
struct DockerOptions;

/// containerd's drop-in in a sandbox.
#[derive(Template)]
#[template(path = "sandbox/containerd.conf.jinja", ext = "txt")]
struct ContainerdOptions;

/// The mount unit of the Docker store disk.
#[derive(Template)]
#[template(path = "sandbox/docker-store.mount.jinja", ext = "txt")]
struct DockerStoreMount;

/// The mount unit of the config drive.
#[derive(Template)]
#[template(path = "sandbox/config-drive.mount.jinja", ext = "txt")]
struct ConfigDriveMount;

/// The unit that names and addresses the NICs.
#[derive(Template)]
#[template(path = "sandbox/sherpa-links.service.jinja", ext = "txt")]
struct LinksUnit;

/// The unit that starts the node's container.
#[derive(Template)]
#[template(path = "sandbox/sherpa-node.service.jinja", ext = "txt")]
struct NodeUnit;

/// The unit of the network OS CLI on the serial console.
#[derive(Template)]
#[template(path = "sandbox/sherpa-console.service.jinja", ext = "txt")]
struct ConsoleUnit;

/// The unit that loads the image archive into a Docker store.
#[derive(Template)]
#[template(path = "sandbox/sherpa-store.service.jinja", ext = "txt")]
struct StoreUnit;

/// The script that names and addresses the NICs.
#[derive(Template)]
#[template(path = "sandbox/links.sh.jinja", ext = "txt")]
struct LinksScript;

/// The script of the serial console.
#[derive(Template)]
#[template(path = "sandbox/console.sh.jinja", ext = "txt")]
struct ConsoleScript;

/// The script that writes a status line to the status channel.
#[derive(Template)]
#[template(path = "sandbox/report.sh.jinja", ext = "txt")]
struct ReportScript;

/// The script that loads the image archive into a Docker store.
#[derive(Template)]
#[template(path = "sandbox/store.sh.jinja", ext = "txt")]
struct StoreScript;

/// The node's container name, for the scripts that run it.
#[derive(Template)]
#[template(path = "sandbox/container.jinja", ext = "txt")]
struct ContainerFile<'a> {
    name: &'a str,
}

/// The NICs or veths the guest names or makes, one `mac name` line each.
#[derive(Template)]
#[template(path = "sandbox/interfaces.jinja", ext = "txt")]
struct InterfacesFile<'a> {
    interfaces: &'a [GuestInterface],
}

/// The management addresses, one `interface address/prefix gateway` line each.
#[derive(Template)]
#[template(path = "sandbox/management.jinja", ext = "txt")]
struct ManagementFile<'a> {
    addresses: &'a [GuestAddress],
}

/// The sandbox's own NIC: its MAC on the first line, then one `address/prefix gateway` line per
/// address.
#[derive(Template)]
#[template(path = "sandbox/sandbox-nic.jinja", ext = "txt")]
struct SandboxNicFile<'a> {
    mac: &'a str,
    addresses: &'a [GuestAddress],
}

/// The writable volumes to copy into the container, one `source destination` line each.
#[derive(Template)]
#[template(path = "sandbox/writable.jinja", ext = "txt")]
struct WritableFile<'a> {
    volumes: &'a [WritableVolume],
}

/// The console's command, one argument per line.
#[derive(Template)]
#[template(path = "sandbox/console-command.jinja", ext = "txt")]
struct ConsoleCommandFile<'a> {
    command: &'a [&'a str],
}

/// The container's `docker create` arguments, each followed by a NUL byte, which no argument
/// holds.
#[derive(Template)]
#[template(path = "sandbox/create.jinja", ext = "txt")]
struct CreateFile<'a> {
    args: &'a [String],
}

impl NodeScript<'static> {
    /// The script as a sandbox runs it, with the files sherpad gives the guest.
    #[must_use]
    pub fn guest() -> Self {
        Self {
            container: CONTAINER_FILE,
            create: CREATE_FILE,
            writable: WRITABLE_FILE,
            report: REPORT_SCRIPT_PATH,
            created: CREATED_MARKER,
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

impl SandboxGuest<'_> {
    /// The Ignition config of the sandbox's first boot, as JSON.
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
                SANDBOX_NIC_FILE,
                FILE_MODE,
                &SandboxNicFile {
                    mac: &self.own_nic.mac,
                    addresses: &self.own_nic.addresses,
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
                &mount_unit(SANDBOX_CONFIG_DRIVE_MOUNT),
                &ConfigDriveMount.render()?,
            ),
            IgnitionUnit::enabled(LINKS_UNIT, &LinksUnit.render()?),
            IgnitionUnit::enabled(NODE_UNIT, &NodeUnit.render()?),
            IgnitionUnit::enabled(CONSOLE_UNIT, &ConsoleUnit.render()?),
        ]);
        IgnitionConfig::new(vec![], files, vec![], units, vec![], vec![]).to_json_pretty()
    }
}

/// The Ignition config of the VM that makes a Docker store, as JSON: Docker as in a sandbox, on
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
        "serial-getty@{SANDBOX_CONSOLE_TTY}.service"
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
