//! The Ignition configs of sandboxes, the Flatcar VMs that each run one container node, and of
//! the VMs that make their boot disks and Docker stores (`docs/architecture/SANDBOX.md`).

use std::net::{IpAddr, SocketAddr};

use anyhow::Result;
use askama::Template;

use shared::konst::SSH_PORT;

use crate::ignition::{
    IgnitionConfig, IgnitionFile, IgnitionFileSystem, IgnitionUnit, IgnitionUser, KernelArguments,
};

/// The volume label Flatcar's Ignition looks for on a config drive.
pub const SANDBOX_CONFIG_DRIVE_LABEL: &str = "config-2";
/// Where Ignition reads its config on a config drive.
pub const SANDBOX_CONFIG_DRIVE_USER_DATA: &str = "openstack/latest/user_data";
/// Where a sandbox mounts its config drive.
pub const SANDBOX_CONFIG_DRIVE_MOUNT: &str = "/media/config";
/// Where the body Docker creates a sandbox's container from is on its config drive.
pub const SANDBOX_CONTAINER_BODY: &str = "sherpa/container.json";
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
/// The kernel arguments a sandbox boots with beyond those of Flatcar's bootloader: the kernel
/// prints only errors on the serial console, the network OS's.
pub const SANDBOX_KERNEL_ARGUMENTS: [&str; 1] = ["quiet"];
/// The only login on the sandbox's own sshd.
pub const SANDBOX_LOGIN: &str = "core";

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
const SANDBOX_NIC_ADDRESSES_FILE: &str = "/etc/sherpa/sandbox-nic-addresses";
const CONSOLE_COMMAND_FILE: &str = "/etc/sherpa/console";
const WRITABLE_FILE: &str = "/etc/sherpa/writable";
const DOCKER_SOCKET: &str = "/run/docker.sock";
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
const SSHD_UNIT: &str = "sshd.service";
const SSHD_CONFIG_PATH: &str = "/etc/ssh/sshd_config.d/00-sherpa.conf";
const SSHD_DROP_IN_PATH: &str = "/etc/systemd/system/sshd.service.d/10-sherpa.conf";
const SSHD_CONFIG_MODE: u32 = 0o600;
const FILE_MODE: u32 = 0o644;
const SCRIPT_MODE: u32 = 0o755;
const MASKED_UNITS: [&str; 16] = [
    "update-engine.service",
    "locksmithd.service",
    "sshd.socket",
    "sshd-vsock.socket",
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
    pub ssh_keys: &'a [String],
    pub console_command: &'a [&'a str],
    pub docker_api_version: &'a str,
    pub writable: &'a [WritableVolume],
}

/// The script that creates the node's container on its first boot and starts it on every boot.
#[derive(Debug, Template)]
#[template(path = "sandbox/node.sh.jinja", ext = "txt")]
pub struct NodeScript {
    pub api_version: String,
    pub socket: String,
    pub container: String,
    pub body: String,
    pub writable: String,
    pub report: String,
    pub created: String,
}

/// Docker's drop-in in a sandbox.
#[derive(Template)]
#[template(path = "sandbox/docker.conf.jinja", ext = "txt")]
struct DockerOptions;

/// containerd's config in a sandbox.
#[derive(Template)]
#[template(path = "sandbox/containerd.toml.jinja", ext = "txt")]
struct ContainerdConfig;

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

/// The config of the sandbox's own sshd, listening on `listen` only.
#[derive(Template)]
#[template(path = "sandbox/sshd.conf.jinja", ext = "txt")]
struct SshdConfig<'a> {
    listen: &'a [SocketAddr],
}

/// sshd's drop-in, which runs it in the sandbox's own network namespace once that has its
/// addresses.
#[derive(Template)]
#[template(path = "sandbox/sshd.service.conf.jinja", ext = "txt")]
struct SshdOptions;

/// The script that names and addresses the NICs.
#[derive(Template)]
#[template(path = "sandbox/links.sh.jinja", ext = "txt")]
struct LinksScript;

/// The script of the serial console.
#[derive(Template)]
#[template(path = "sandbox/console.sh.jinja", ext = "txt")]
struct ConsoleScript;

/// The script that writes a status line to the status channel `channel`, and that the other
/// scripts source for `fail`.
#[derive(Debug, Template)]
#[template(path = "sandbox/report.sh.jinja", ext = "txt")]
pub struct ReportScript {
    pub channel: String,
}

/// The script that loads the image archive into a Docker store.
#[derive(Template)]
#[template(path = "sandbox/store.sh.jinja", ext = "txt")]
struct StoreScript;

/// The NICs or veths the guest names or makes, one `mac name` line each.
#[derive(Template)]
#[template(path = "sandbox/interfaces.jinja", ext = "txt")]
struct InterfacesFile<'a> {
    interfaces: &'a [GuestInterface],
}

/// The addresses of a NIC, one `address/prefix gateway` line each.
#[derive(Template)]
#[template(path = "sandbox/addresses.jinja", ext = "txt")]
struct AddressesFile<'a> {
    addresses: &'a [GuestAddress],
}

impl ReportScript {
    /// The script as a sandbox or a Docker store VM runs it, on the VM's status channel.
    #[must_use]
    pub fn guest() -> Self {
        Self {
            channel: format!("/dev/virtio-ports/{SANDBOX_STATUS_CHANNEL}"),
        }
    }
}

impl NodeScript {
    /// The script as a sandbox runs it, with the files sherpad gives the guest, calling the
    /// Engine API at `api_version`.
    #[must_use]
    pub fn guest(api_version: &str) -> Self {
        Self {
            api_version: api_version.to_owned(),
            socket: DOCKER_SOCKET.to_owned(),
            container: CONTAINER_FILE.to_owned(),
            body: format!("{SANDBOX_CONFIG_DRIVE_MOUNT}/{SANDBOX_CONTAINER_BODY}"),
            writable: WRITABLE_FILE.to_owned(),
            report: REPORT_SCRIPT_PATH.to_owned(),
            created: CREATED_MARKER.to_owned(),
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
            IgnitionFile::root(CONTAINER_FILE, FILE_MODE, &format!("{}\n", self.container)),
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
                &AddressesFile {
                    addresses: self.management,
                }
                .render()?,
            ),
            IgnitionFile::root(
                SANDBOX_NIC_FILE,
                FILE_MODE,
                &format!("{}\n", self.own_nic.mac.to_ascii_lowercase()),
            ),
            IgnitionFile::root(
                SANDBOX_NIC_ADDRESSES_FILE,
                FILE_MODE,
                &AddressesFile {
                    addresses: &self.own_nic.addresses,
                }
                .render()?,
            ),
            IgnitionFile::root(
                CONSOLE_COMMAND_FILE,
                FILE_MODE,
                &lines(self.console_command.iter().map(|arg| (*arg).to_owned())),
            ),
            IgnitionFile::root(
                WRITABLE_FILE,
                FILE_MODE,
                &lines(
                    self.writable
                        .iter()
                        .map(|volume| format!("{} {}", volume.source, volume.destination)),
                ),
            ),
            IgnitionFile::root(LINKS_SCRIPT_PATH, SCRIPT_MODE, &LinksScript.render()?),
            IgnitionFile::root(
                NODE_SCRIPT_PATH,
                SCRIPT_MODE,
                &NodeScript::guest(self.docker_api_version).render()?,
            ),
            IgnitionFile::root(CONSOLE_SCRIPT_PATH, SCRIPT_MODE, &ConsoleScript.render()?),
        ]);
        files.extend(self.sshd_files()?);
        let mut units = docker_units()?;
        units.extend([
            IgnitionUnit::enabled(
                &mount_unit(SANDBOX_CONFIG_DRIVE_MOUNT),
                &ConfigDriveMount.render()?,
            ),
            IgnitionUnit::enabled(LINKS_UNIT, &LinksUnit.render()?),
            IgnitionUnit::enabled(NODE_UNIT, &NodeUnit.render()?),
            IgnitionUnit::enabled(CONSOLE_UNIT, &ConsoleUnit.render()?),
            IgnitionUnit {
                name: SSHD_UNIT.to_owned(),
                enabled: Some(true),
                ..Default::default()
            },
        ]);
        IgnitionConfig::new(vec![self.login()], files, vec![], units, vec![], vec![])
            .to_json_pretty()
    }

    /// The config and drop-in of the sandbox's own sshd, on port 22 of its own addresses.
    fn sshd_files(&self) -> Result<[IgnitionFile; 2]> {
        let listen: Vec<SocketAddr> = self
            .own_nic
            .addresses
            .iter()
            .map(|address| SocketAddr::new(address.address, SSH_PORT))
            .collect();
        Ok([
            IgnitionFile::root(
                SSHD_CONFIG_PATH,
                SSHD_CONFIG_MODE,
                &SshdConfig { listen: &listen }.render()?,
            ),
            IgnitionFile::root(SSHD_DROP_IN_PATH, FILE_MODE, &SshdOptions.render()?),
        ])
    }

    /// The sandbox's `core` login, with the owner's keys and no password.
    fn login(&self) -> IgnitionUser {
        IgnitionUser {
            name: SANDBOX_LOGIN.to_owned(),
            password_hash: None,
            home_dir: None,
            shell: None,
            ssh_authorized_keys: self.ssh_keys.to_vec(),
            groups: vec![],
        }
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
    units.extend([
        IgnitionUnit::masked(SSHD_UNIT),
        IgnitionUnit::enabled(STORE_UNIT, &StoreUnit.render()?),
    ]);
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

/// The Ignition config of the VM that makes the sandboxes' boot disk, as JSON: only the
/// sandboxes' kernel arguments, which Flatcar writes to the disk's OEM partition before a reboot
/// that stops the VM.
///
/// # Errors
///
/// Fails when the config cannot be serialized.
pub fn boot_vm_ignition_json() -> Result<String> {
    let mut config = IgnitionConfig::new(vec![], vec![], vec![], vec![], vec![], vec![]);
    config.kernel_arguments = Some(KernelArguments {
        should_exist: SANDBOX_KERNEL_ARGUMENTS.map(str::to_owned).to_vec(),
    });
    config.to_json_pretty()
}

/// The files of Docker and containerd, of updates off, and of the status report, in every
/// Flatcar VM Sherpa runs for a node.
fn docker_files() -> Result<Vec<IgnitionFile>> {
    Ok(vec![
        IgnitionFile::disable_updates(),
        IgnitionFile::root(DOCKER_DROP_IN_PATH, FILE_MODE, &DockerOptions.render()?),
        IgnitionFile::root(
            CONTAINERD_CONFIG_PATH,
            FILE_MODE,
            &ContainerdConfig.render()?,
        ),
        IgnitionFile::root(
            CONTAINERD_DROP_IN_PATH,
            FILE_MODE,
            &ContainerdOptions.render()?,
        ),
        IgnitionFile::root(
            REPORT_SCRIPT_PATH,
            SCRIPT_MODE,
            &ReportScript::guest().render()?,
        ),
    ])
}

/// The units of Docker on its store disk, with updates, Flatcar's SSH sockets, the serial getty,
/// metadata, networking and the first boot's cache rebuilds off.
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

/// `items`, each on a line of its own.
fn lines(items: impl Iterator<Item = String>) -> String {
    items.map(|item| item + "\n").collect()
}

/// The name of the mount unit of absolute path `path`, which holds no character systemd
/// escapes but `/`.
fn mount_unit(path: &str) -> String {
    format!("{}.mount", path.trim_start_matches('/').replace('/', "-"))
}
