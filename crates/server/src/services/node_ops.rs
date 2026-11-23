//! Per-node operations shared by `up`, `redeploy` and `resume`: configuration, domains and
//! starting nodes.

use std::collections::HashSet;
use std::fs;
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use askama::Template;
use jato::{Auth, ConnectOptions, Hop, HostKeyPolicy, Node, Session};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tracing::instrument;

use crate::konst::{
    KEYCLOAK_HTTPS_PORT, LAB_CERTS_DIR, NODE_CONFIG_DIR_MODE, NODE_CONFIG_FILE_MODE,
    SHERPA_BLANK_DISK_DIR, TFTP_CONFIG_FILE_MODE, VAULT_API_PORT, ZTP_ISO,
};
use crate::services::container_networks::is_node_network;
use crate::services::error::ServiceError;
use crate::services::guest_account::{self, GuestAccount, KeyFormat, PasswordForm};
use crate::services::image_lifecycle::ImageLease;
use crate::services::keycloak::api::keycloak_address;
use crate::services::keycloak::lab::{KEYCLOAK_DIR, ensure_keycloak_credentials};
use crate::services::p2p_links::{KernelHost, LinkHost};
use crate::services::progress::ProgressSender;
use crate::services::surreal_db::{
    SURREAL_DB_DIR, ensure_surreal_db_credentials, surreal_db_address,
};
use crate::services::talos::secrets::CLUSTERS_DIR;
use crate::services::vault::lab::VAULT_DIR;
use crate::tls::lab_ca::NodeCertPaths;

use container::ContainerSpec;
use db::DevicePassword;
use iso::create_iso;
use libvirt::{ConsoleSockets, Qemu};
use network::NetnsVethState;
use shared::data;
use shared::data::StatusKind;
use shared::konst::{
    CONTAINER_DISK_NAME, KVM_OUI, NODE_CLOUD_INIT_SUDO_RULE, NODE_CONFIGS_DIR, NODE_USER_ENV_FILE,
    SHERPA_DOMAIN_NAME, SHERPA_SSHD_READY_MARKER, SHERPA_STORAGE_POOL, SHERPA_STORAGE_POOL_PATH,
    SHERPA_USERNAME, SSH_PORT, TALOS_API_PORT, TFTP_DIR, UNIKERNEL_DISK_FILENAME,
    UNIKERNEL_KERNEL_FILENAME, VM_DISK_FILENAME, ZTP_DIR,
};
use shared::util;
use shared::validators::{effective_data_interface_count, effective_ztp, mode_from_octal_digits};
use template::CryptHash;
use topology::{NodeReadyCheck, SystemdUnitData};
use virt::error::ErrorNumber;
use virt::sys::VIR_DOMAIN_UNDEFINE_NVRAM;

const SHERPA_BLANK_DISK_FAT32: &str = "fat32.img";
const SHERPA_BLANK_DISK_IOSV: &str = "iosv.img";
const SHERPA_BLANK_DISK_ISE: &str = "ise.img";
const SHERPA_BLANK_DISK_JUNOS: &str = "junos.img";
const SHERPA_BLANK_DISK_EXT4_500MB: &str = "ext4-500mb.img";
const SHERPA_ISOLATED_NETWORK_NAME: &str = "sherpa-isolated";
const SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX: &str = "bri";
const SHERPA_RESERVED_NETWORK_NAME: &str = "sherpa-reserved";
const SHERPA_RESERVED_NETWORK_BRIDGE_PREFIX: &str = "brr";
const CLOUD_INIT_USER_DATA: &str = "user-data";
const CLOUD_INIT_META_DATA: &str = "meta-data";
const CLOUD_INIT_NETWORK_CONFIG: &str = "network-config";
/// Volume label of the ZTP ISO. cloud-init's `NoCloud` source finds its seed by this label.
const ZTP_ISO_VOLUME_ID: &str = "cidata";
/// Volume label PAN-OS looks for on its bootstrap ISO.
const PALOALTO_BOOTSTRAP_VOLUME_ID: &str = "bootstrap";
const ZTP_JSON: &str = "ztp.json";
const CISCO_IOSXE_ZTP_CONFIG: &str = "iosxe_config.txt";
const CISCO_IOS_ZTP_CONFIG: &str = "ios_config.txt";
const CISCO_ASAV_ZTP_CONFIG: &str = "day0-config";
const CISCO_NXOS_ZTP_CONFIG: &str = "nxos_config.txt";
const CISCO_IOSXR_ZTP_CONFIG: &str = "iosxr_config.txt";
const CISCO_ISE_ZTP_CONFIG: &str = "ise-ztp.conf";
const CISCO_FTDV_ZTP_CONFIG: &str = "day0-config";
const CUMULUS_ZTP: &str = "cumulus-ztp";
const ARISTA_ABOOT_DIR: &str = "aboot_iso";
/// Where a cEOS node's ZTP config is mounted, read-only. EOS saves by replacing
/// `ARISTA_CEOS_STARTUP_CONFIG`, which a bind-mounted file cannot be, so the config is copied
/// there when the container starts.
const ARISTA_CEOS_ZTP_VOLUME_MOUNT: &str = "/mnt/sherpa-ztp/startup-config";
const ARISTA_CEOS_STARTUP_CONFIG: &str = "/mnt/flash/startup-config";
const NOKIA_SRLINUX_ZTP_VOLUME_MOUNT: &str = "/etc/opt/srlinux/config.json";
const JUNIPER_ZTP_CONFIG: &str = "juniper.conf";
const JUNIPER_ZTP_CONFIG_TGZ: &str = "vmm-config.tgz";
const MIKROTIK_CHR_ZTP_CONFIG: &str = "config.rsc";
const PALOALTO_ZTP_CONFIG: &str = "init-cfg.txt";
const PALOALTO_BOOTSTRAP_CONFIG: &str = "bootstrap.xml";
const NODE_CERTS_DIR: &str = "/etc/sherpa/certs";
const NODE_CERTS_DIR_WINDOWS: &str = r"C:\sherpa\certs";
/// The built-in Administrators group, as `icacls` takes a SID.
const WINDOWS_ADMINISTRATORS_SID: &str = "*S-1-5-32-544";
/// The `LocalSystem` account, as `icacls` takes a SID.
const WINDOWS_SYSTEM_SID: &str = "*S-1-5-18";
const WINDOWS_SSHD_CONFIG: &str = r"C:\ProgramData\ssh\sshd_config";
/// Each account's SSH public keys, in a file named after the account.
const WINDOWS_AUTHORIZED_KEYS_DIR: &str = r"C:\ProgramData\ssh\authorized_keys";
/// The script that disables every enabled local account but the owner's, run once at first boot.
const WINDOWS_OWNER_ONLY_LOGIN_SCRIPT: &str = r"C:\sherpa\owner_only_login.ps1";
/// [`WINDOWS_AUTHORIZED_KEYS_DIR`] as `sshd_config` names it.
const WINDOWS_SSHD_AUTHORIZED_KEYS_DIR: &str = "__PROGRAMDATA__/ssh/authorized_keys";
const NODE_STARTUP_SCRIPTS_DIR: &str = "/opt/sherpa/startup_scripts";
/// doas configuration file (OpenBSD, whose doas has no drop-in directory).
const NODE_DOAS_CONF_FILE: &str = "/etc/doas.conf";
/// Root-only directory where cloud-init stages files until their owners exist.
const NODE_STAGING_DIR: &str = "/opt/sherpa/staging";
/// Guest account database, read at boot to find a user's home directory.
const NODE_PASSWD_FILE: &str = "/etc/passwd";
/// Seconds a readiness check waits for a node's SSH server to finish the SSH handshake.
const SSH_READY_HANDSHAKE_TIMEOUT: u64 = 5;
/// The most an SSH server may send before its identification line: RFC 4253 lets it send
/// other lines first, and the identification line is at most 255 bytes.
const SSH_IDENTIFICATION_MAX_BYTES: u64 = 8192;
const CONTAINER_NOKIA_SRLINUX_ENV_VARS: &[&str] = &["SRLINUX=1"];
const CONTAINER_NOKIA_SRLINUX_USER: &str = "0:0";
const CONTAINER_NOKIA_SRLINUX_COMMANDS: &[&str] = &["bash", "/opt/srlinux/bin/sr_linux"];
/// Docker's default capabilities run the IOL launcher; the entrypoint also removes Docker's
/// address from `eth0`, which IOL owns.
const CONTAINER_CISCO_IOL_CAPABILITIES: &[&str] = &["NET_ADMIN"];
/// The directory the IOL launcher reads the startup config from.
const CISCO_IOL_CONFIG_DIR: &str = "/config";
/// The startup config's name in `CISCO_IOL_CONFIG_DIR`.
const CISCO_IOL_STARTUP_CONFIG: &str = "ios_config.txt";
/// IOL's NVRAM directory, kept in the node's lab folder so a saved config survives a cold boot.
const CISCO_IOL_RUN_DIR: &str = "/tmp/run";
/// The highest IOL application ID Sherpa gives a node. IOL derives its MAC addresses from the
/// ID, and the launcher takes 1000 for itself.
const CISCO_IOL_MAX_APP_ID: u16 = 999;
/// The directory the cJunosEvolved entrypoint reads a startup `juniper.conf` from.
const JUNIPER_CEVOLVED_CONFIG_DIR: &str = "/home/evo/configdisk";
const CONTAINER_ARISTA_CEOS_ENV_VARS: &[&str] = &[
    "INTFTYPE=eth",
    "ETBA=1",
    "SKIP_ZEROTOUCH_BARRIER_IN_SYSDBINIT=1",
    "CEOS=1",
    "EOS_PLATFORM=ceoslab",
    "MAPETH0:1",
    "MGMT_INTF:eth0",
];
const CONTAINER_ARISTA_CEOS_COMMANDS: &[&str] = &[
    "/sbin/init",
    "systemd.setenv=INTFTYPE=eth",
    "systemd.setenv=ETBA=1",
    "systemd.setenv=SKIP_ZEROTOUCH_BARRIER_IN_SYSDBINIT=1",
    "systemd.setenv=CEOS=1",
    "systemd.setenv=EOS_PLATFORM=ceoslab",
    "systemd.setenv=container=docker",
    "systemd.setenv=MAPETH0=1",
    "systemd.setenv=MGMT_INTF=eth0",
];
const CONTAINER_FRR_ENV_VARS: &[&str] = &[];
const FRR_ZTP_DAEMONS_MOUNT: &str = "/etc/frr/daemons";
const FRR_ZTP_CONFIG_MOUNT: &str = "/etc/frr/frr.conf";
const FRR_ZTP_STARTUP_MOUNT: &str = "/tmp/sherpa-init.sh";
/// Where a `sonic_linux` container's start files are mounted, read-only. None goes at
/// `/etc/sonic/config_db.json`: the image's start script renames a file over that path, which a
/// bind mount cannot take.
const SONIC_CONTAINER_SUPERVISORD_MOUNT: &str = "/mnt/sherpa-ztp/supervisord.conf";
const SONIC_CONTAINER_INIT_MOUNT: &str = "/mnt/sherpa-ztp/sherpa-init.sh";
const SONIC_CONTAINER_CONFIG_DB_MOUNT: &str = "/mnt/sherpa-ztp/config_db.json";
/// The image's entrypoint is `supervisord`, so the command is its arguments: Sherpa's config,
/// whose only program is the start script.
const CONTAINER_SONIC_COMMANDS: &[&str] = &["-c", SONIC_CONTAINER_SUPERVISORD_MOUNT];
const CONTAINER_GITLAB_CE_COMMANDS: &[&str] = &[];
const CONTAINER_GITLAB_CE_SHM_SIZE: i64 = 256;
/// `SurrealDB` starts in memory; its root user comes from the environment
/// (`surreal_db_env_vars`), so it is not in the command line every host user can read. It logs
/// at `info`: at `trace` it logs request bodies, so a `/signin` password reached `docker logs`.
const CONTAINER_SURREAL_DB_COMMANDS: &[&str] = &["start", "--log", "info", "memory"];
const CONTAINER_MONGO_DB_COMMANDS: &[&str] = &[];
const CONTAINER_NATS_COMMANDS: &[&str] = &[];
const CONTAINER_FORGEJO_COMMANDS: &[&str] = &[];
/// Environment of a Vault node running a config the user supplied.
const CONTAINER_HASHICORP_VAULT_ENV_VARS: &[&str] = &["VAULT_ADDR=http://127.0.0.1:8200"];
/// Keycloak starts in production mode; the image's entrypoint is `kc.sh`.
const CONTAINER_KEYCLOAK_COMMANDS: &[&str] = &["start"];
/// Keycloak runs as root, so it can read the lab certificate key mounted from the server.
const CONTAINER_KEYCLOAK_USER: &str = "0:0";
const VAULT_CLUSTER_PORT: u16 = 8201;
/// Vault's start command. It replaces the image's entrypoint, which would drop to the `vault`
/// user, so Vault runs as root and can read the lab certificate key mounted from the server.
const CONTAINER_HASHICORP_VAULT_COMMANDS: &[&str] = &[
    "sh",
    "-c",
    "mkdir -p /vault/data && exec vault server -config=/vault/config/config.hcl",
];
const CONTAINER_HASHICORP_VAULT_CAPABILITIES: &[&str] = &["IPC_LOCK"];
const VAULT_ZTP_CONFIG_MOUNT: &str = "/vault/config/config.hcl";

/// The folders in a lab folder that hold the lab's own files. A container named like one of them
/// writes its config files there too, so that folder is never removed as the node's.
pub(crate) const LAB_OWN_DIRS: [&str; 6] = [
    LAB_CERTS_DIR,
    ZTP_DIR,
    CLUSTERS_DIR,
    VAULT_DIR,
    KEYCLOAK_DIR,
    SURREAL_DB_DIR,
];

/// The folder in a lab folder holding each VM's boot files, `<node>/`, until its media are in the
/// storage pool. Node names have no `.`, so no node's folder can be named like it.
const VM_BOOT_DIR: &str = "vm-boot.d";

// ============================================================================
// Result structs
/// What generating a lab's node configurations needs: its folders and management network, the
/// accounts, and where progress goes.
#[derive(Clone, Copy)]
pub(crate) struct LabZtp<'a> {
    pub(crate) lab_id: &'a str,
    pub(crate) lab_dir: &'a str,
    pub(crate) images_dir: &'a str,
    pub(crate) dns: &'a data::Dns,
    pub(crate) mgmt_net: &'a data::SherpaNetwork,
    pub(crate) progress: &'a ProgressSender,
    /// The lab owner's account; each node gets its own form of it, if it takes one, and the
    /// fixed accounts of a node take its device password.
    pub(crate) owner_account: &'a GuestAccount,
}

/// One VM's part of generating its configuration: the lab's, plus the node's addresses,
/// certificate and account. Every ZTP method's generator takes it.
#[derive(Clone, Copy)]
struct NodeZtp<'a> {
    lab_dir: &'a str,
    /// The folder the VM's boot files are written to, `vm_boot_dir`.
    boot_dir: &'a str,
    tftp_dir: &'a str,
    images_dir: &'a str,
    /// The lab owner's device password, for nodes whose only account is a fixed one.
    device_password: &'a DevicePassword,
    dns: &'a data::Dns,
    mgmt_net: &'a data::SherpaNetwork,
    progress: &'a ProgressSender,
    node_ipv4_address: Ipv4Addr,
    mac_address: &'a str,
    node_name_with_lab: &'a str,
    cert_paths: Option<&'a NodeCertPaths>,
    guest_account: Option<&'a GuestAccount>,
}

/// What a container model's setup needs: the lab's and the node's folders, the node's custom
/// config, the lab's accounts and management network, and the node's address and certificate.
#[derive(Clone, Copy)]
struct ContainerZtp<'a> {
    /// The lab folder, where the lab's application nodes' admin credentials are kept.
    lab_dir: &'a str,
    dir: &'a str,
    /// The lab owner's login, given a user on a Keycloak node.
    owner: &'a str,
    custom_ztp: Option<&'a str>,
    guest_account: Option<&'a GuestAccount>,
    dns: &'a data::Dns,
    mgmt_net: &'a data::SherpaNetwork,
    node_ipv4_address: Ipv4Addr,
    cert_paths: Option<&'a NodeCertPaths>,
}

/// A container's settings: the manifest's, then changed by its model's setup.
struct ContainerSetup {
    image: String,
    env_vars: Vec<String>,
    volumes: Vec<String>,
    commands: Vec<String>,
    capabilities: Vec<String>,
    privileged: bool,
    shm_size: Option<i64>,
    user: Option<String>,
}

/// A VM's configuration media, each a source in the lab folder and its destination in the
/// storage pool.
#[derive(Default)]
struct VmZtpMedia {
    cdrom: Option<(String, String)>,
    config_disk: Option<(String, String)>,
    usb_disk: Option<(String, String)>,
    ignition: Option<(String, String)>,
}

// ============================================================================

pub struct ContainerZtpResult {
    pub image: String,
    pub env_vars: Vec<String>,
    pub volumes: Vec<String>,
    pub commands: Vec<String>,
    pub capabilities: Vec<String>,
    pub privileged: bool,
    pub shm_size: Option<i64>,
    pub user: Option<String>,
    pub ztp_record: data::ZtpRecord,
}

pub struct VmZtpResult {
    pub ztp_record: data::ZtpRecord,
    pub clone_disks: Vec<data::CloneDisk>,
    pub disks: Vec<data::NodeDisk>,
    pub mac_address: String,
    pub firmware: VmFirmware,
}

/// What a VM's model gives its guest through the firmware, beyond its disks.
#[derive(Debug, Default)]
pub struct VmFirmware {
    /// Extra QEMU arguments, the SMBIOS settings of Juniper models.
    pub qemu_commands: Vec<data::QemuCommand>,
    /// Flatcar's Ignition config in the storage pool, the domain's firmware config entry.
    pub ignition_config: Option<String>,
}

/// What a virtual machine's readiness check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmReadiness {
    NotReady,
    /// Ready, with the host key the node offered when readiness was an SSH handshake.
    Ready {
        host_key: Option<String>,
    },
}

/// The disks, interfaces and networks a VM or unikernel domain is built from.
pub struct DomainParts {
    pub disks: Vec<data::NodeDisk>,
    pub interfaces: Vec<data::Interface>,
    /// The UNIX sockets of the node's serial and VNC consoles (`console::console_sockets`). A
    /// unikernel has no VNC console, and binds only the serial one.
    pub consoles: ConsoleSockets,
    pub management_network: String,
    /// Empty when the node has no isolated network.
    pub isolated_network: String,
    /// Empty when the node has no reserved network.
    pub reserved_network: String,
}

/// A file cloud-init writes as root to the staging directory, then moves into place from
/// `runcmd` once its owner exists. This avoids `write_files` `defer`, which some images'
/// cloud-init configs never run.
#[derive(Debug)]
struct StagedFile {
    /// Absolute path, or `~/`-relative to the user's home, which is looked up at boot because
    /// it differs by OS (`/usr/home` on FreeBSD).
    dst: String,
    content: String,
    encoding: Option<String>,
    user: String,
    /// `None` uses the user's primary group, which differs between distributions.
    group: Option<String>,
    /// Octal mode for `chmod`, such as `600`.
    permissions: String,
}

// ============================================================================
// Helper functions
// ============================================================================

pub fn node_isolated_network_data(
    node_name: &str,
    node_index: u16,
    lab_id: &str,
) -> data::LabIsolatedNetwork {
    data::LabIsolatedNetwork {
        network_name: format!("{SHERPA_ISOLATED_NETWORK_NAME}-{node_name}-{lab_id}"),
        bridge_name: format!("{SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX}{node_index}-{lab_id}"),
    }
}

pub fn node_reserved_network_data(
    node_name: &str,
    node_index: u16,
    lab_id: &str,
) -> data::LabReservedNetwork {
    data::LabReservedNetwork {
        network_name: format!("{SHERPA_RESERVED_NETWORK_NAME}-{node_name}-{lab_id}"),
        bridge_name: format!("{SHERPA_RESERVED_NETWORK_BRIDGE_PREFIX}{node_index}-{lab_id}"),
    }
}

/// Write the `NoCloud` seed files for a node into `dir`.
///
/// `meta-data` names the node and `network-config` gives its management interface a static
/// address, gateway and DNS server. `user_data` is written byte for byte when present; when it
/// is absent any earlier `user-data` is removed, so the node boots without a config (Talos
/// Linux then waits in maintenance mode).
#[tracing::instrument(level = "debug", skip(mgmt_net, user_data), fields(%hostname))]
pub(crate) fn write_nocloud_files(
    dir: &str,
    hostname: &str,
    node_ipv4_address: Ipv4Addr,
    mac_address: &str,
    mgmt_net: &data::SherpaNetwork,
    node_ipv6_address: Option<Ipv6Addr>,
    user_data: Option<&str>,
) -> Result<()> {
    util::create_dir_with_permissions(Path::new(dir), NODE_CONFIG_DIR_MODE)?;

    let meta_data = template::MetaDataConfig {
        instance_id: format!("iid-{hostname}"),
        local_hostname: hostname.to_owned(),
        ..Default::default()
    };
    util::write_file_with_permissions(
        Path::new(&format!("{dir}/{CLOUD_INIT_META_DATA}")),
        meta_data.render()?.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    let network_config = template::CloudInitNetwork::ztp_interface(
        node_ipv4_address,
        mac_address.to_owned(),
        &mgmt_net.v4,
        node_ipv6_address,
        mgmt_net.v6.as_ref(),
    );
    util::write_file_with_permissions(
        Path::new(&format!("{dir}/{CLOUD_INIT_NETWORK_CONFIG}")),
        network_config.render()?.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    let user_data_path = format!("{dir}/{CLOUD_INIT_USER_DATA}");
    match user_data {
        Some(content) => {
            util::write_file_with_permissions(
                Path::new(&user_data_path),
                content.as_bytes(),
                NODE_CONFIG_FILE_MODE,
            )?;
        }
        None => match fs::remove_file(&user_data_path) {
            Ok(()) => tracing::debug!(path = %user_data_path, "Removed stale user-data"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %user_data_path, "No user-data to remove");
            }
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to remove {user_data_path}"));
            }
        },
    }
    Ok(())
}

/// Build the `NoCloud` `cidata` ISO for a node.
#[instrument(level = "debug", skip_all, fields(node_name = %node.name))]
fn generate_nocloud_ztp(
    node: &topology::NodeExpanded,
    ztp: &NodeZtp<'_>,
    user_data: Option<&str>,
) -> Result<()> {
    let NodeZtp {
        mgmt_net,
        node_ipv4_address,
        mac_address,
        progress,
        ..
    } = *ztp;
    let dir = ztp.boot_dir.to_owned();
    if user_data.is_none() {
        let _ = progress.send_status(
            format!(
                "No config for node {}; it will wait for one in maintenance mode",
                node.name
            ),
            StatusKind::Info,
        );
    }
    write_nocloud_files(
        &dir,
        &node.name,
        node_ipv4_address,
        mac_address,
        mgmt_net,
        node.ipv6_address,
        user_data,
    )?;
    create_iso(&dir, format!("{dir}/{ZTP_ISO}"), ZTP_ISO_VOLUME_ID)?;
    Ok(())
}

/// The index of the next disk on a node, for its target device name.
fn disk_index(disks: &[data::NodeDisk]) -> Result<u8> {
    u8::try_from(disks.len()).context("Too many disks for one node")
}

/// Map a node model to its ZTP config filename for CDROM-based ZTP.
pub fn ztp_config_filename(model_kind: data::ModelKind) -> Result<String> {
    let name = match model_kind.model() {
        data::NodeModel::CiscoCsr1000
        | data::NodeModel::CiscoCat8000
        | data::NodeModel::CiscoCat9000 => CISCO_IOSXE_ZTP_CONFIG.replace('-', "_"),
        data::NodeModel::CiscoAsa => CISCO_ASAV_ZTP_CONFIG.to_string(),
        data::NodeModel::CiscoNexus9300 => CISCO_NXOS_ZTP_CONFIG.to_string(),
        data::NodeModel::CiscoIosxr9000 => CISCO_IOSXR_ZTP_CONFIG.to_string(),
        data::NodeModel::CiscoFtd => CISCO_FTDV_ZTP_CONFIG.to_string(),
        data::NodeModel::JuniperSrxv3
        | data::NodeModel::JuniperRouter
        | data::NodeModel::JuniperSwitch => JUNIPER_ZTP_CONFIG.to_string(),
        data::NodeModel::PaloaltoPanos => PALOALTO_ZTP_CONFIG.to_string(),
        model => bail!("No ZTP config filename mapping for model: {model}"),
    };
    Ok(name)
}

/// If the node has a custom ZTP config, log and send a progress message, then return the content.
pub fn take_custom_ztp_config(
    node: &mut topology::NodeExpanded,
    progress: &ProgressSender,
) -> Option<String> {
    match node.ztp.config.take() {
        Some(config) => {
            tracing::info!(node_name = %node.name, "Using custom ZTP config for node");
            let _ = progress.send_status(
                format!("Using custom ZTP config for node: {}", node.name),
                StatusKind::Info,
            );
            Some(config)
        }
        None => None,
    }
}

pub fn get_node_data(node_name: &str, data: &[data::NodeSetupData]) -> Result<data::NodeSetupData> {
    Ok(data
        .iter()
        .find(|x| x.name == node_name)
        .ok_or_else(|| anyhow!("Node setup data not found for node: {node_name}"))?
        .clone())
}

// ============================================================================
// Container ZTP Generation
// ============================================================================

/// Return Linux capabilities required by a given container node model.
pub fn model_capabilities(model_kind: data::ModelKind) -> Vec<String> {
    match model_kind.model() {
        data::NodeModel::HashicorpVault => CONTAINER_HASHICORP_VAULT_CAPABILITIES
            .iter()
            .map(std::string::ToString::to_string)
            .collect(),
        data::NodeModel::CiscoIol | data::NodeModel::CiscoIoll2 => CONTAINER_CISCO_IOL_CAPABILITIES
            .iter()
            .map(std::string::ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Generate ZTP configuration for a container node.
/// Returns the image, `env_vars`, volumes, commands, privileged flag, user, and ZTP record.
pub(crate) fn generate_container_ztp(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab: &LabZtp<'_>,
    node_ipv4_address: Ipv4Addr,
    cert_paths: Option<&NodeCertPaths>,
) -> Result<ContainerZtpResult> {
    let LabZtp {
        lab_dir,
        dns,
        mgmt_net,
        progress,
        owner_account,
        ..
    } = *lab;
    let node_account = guest_account::node_owner(owner_account, node, node_image)?;
    let dir = format!("{}/{}", lab_dir, node.name);
    let custom_ztp = take_custom_ztp_config(node, progress);
    let ztp = ContainerZtp {
        lab_dir,
        dir: &dir,
        owner: &owner_account.username,
        custom_ztp: custom_ztp.as_deref(),
        guest_account: node_account.as_ref(),
        dns,
        mgmt_net,
        node_ipv4_address,
        cert_paths,
    };
    let container = node.container.clone().unwrap_or_default();
    let mut setup = ContainerSetup {
        image: node.image.clone().unwrap_or_default(),
        env_vars: node.environment_variables.clone().unwrap_or_default(),
        volumes: Vec::new(),
        commands: container.commands.unwrap_or_default(),
        capabilities: Vec::new(),
        privileged: container.privileged.unwrap_or(false),
        shm_size: container.shm_size,
        user: container.user,
    };

    let setting = effective_ztp(node.ztp.method, node_image);
    container_model_setup(node, node_image, &ztp, setting, &mut setup)?;
    add_container_volumes(node, setting, cert_paths, &mut setup.volumes);

    let ztp_record = data::ZtpRecord {
        node_name: node.name.clone(),
        config_file: format!("{}.conf", node.name),
        ipv4_address: node_ipv4_address,
        ipv6_address: None,
        mac_address: String::new(),
        ztp: setting,
        ssh_port: Some(SSH_PORT),
    };

    Ok(ContainerZtpResult {
        image: setup.image,
        env_vars: setup.env_vars,
        volumes: setup.volumes,
        commands: setup.commands,
        capabilities: setup.capabilities,
        privileged: setup.privileged,
        shm_size: setup.shm_size,
        user: setup.user,
        ztp_record,
    })
}

/// Apply the container's ZTP setting. With ZTP off a container gets its model's start
/// settings alone. `volume` adds the config files Sherpa generates, their mounts and the
/// logins. No container takes another method, and the validator refuses one before a node is
/// planned, so reaching one here means a setting skipped validation.
fn container_model_setup(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &ContainerZtp<'_>,
    setting: data::Ztp,
    setup: &mut ContainerSetup,
) -> Result<()> {
    match setting {
        data::Ztp::Disabled => container_start_settings(node, node_image, ztp, setup),
        data::Ztp::Volume => {
            container_start_settings(node, node_image, ztp, setup)?;
            container_volume_ztp(node, node_image, ztp, setup)
        }
        data::Ztp::CloudInit
        | data::Ztp::Cdrom
        | data::Ztp::Disk
        | data::Ztp::Http
        | data::Ztp::Ignition
        | data::Ztp::NoCloud
        | data::Ztp::Tftp
        | data::Ztp::Usb => bail!(
            "ZTP setting '{setting}' of container node '{}' was not validated: containers take \
             only 'disabled' and 'volume'",
            node.name
        ),
    }
}

/// The settings a container of its model needs to start at all: its environment, user and
/// command, and for Vault its own config. Other models keep the manifest's settings. The image
/// is the one the node resolved to and Docker was checked for; no model replaces it.
fn container_start_settings(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    match node.model_kind.model() {
        data::NodeModel::AristaEos => arista_eos_container_start(node, setup),
        data::NodeModel::NokiaSrlinux => srlinux_start(node, setup),
        data::NodeModel::JuniperEvolved => cevolved_start(node, node_image, setup)?,
        data::NodeModel::CiscoIol | data::NodeModel::CiscoIoll2 => {
            iol_start(node, node_image, ztp, setup)?;
        }
        data::NodeModel::FrrLinux => frr_start(node, setup),
        data::NodeModel::SonicLinux => sonic_container_start(node, node_image, ztp, setup)?,
        data::NodeModel::GitlabCe => {
            app_container(node, setup, CONTAINER_GITLAB_CE_COMMANDS);
            // GitLab CE requires a larger /dev/shm for bundled PostgreSQL
            if setup.shm_size.is_none() {
                setup.shm_size = Some(CONTAINER_GITLAB_CE_SHM_SIZE * 1024 * 1024);
            }
            node.container.get_or_insert_default().shm_size = setup.shm_size;
        }
        data::NodeModel::SurrealDb => surreal_db_container(node, ztp, setup)?,
        data::NodeModel::MongoDb => app_container(node, setup, CONTAINER_MONGO_DB_COMMANDS),
        data::NodeModel::NatsBus => {
            app_container(node, setup, CONTAINER_NATS_COMMANDS);
        }
        data::NodeModel::ForgejoForge => app_container(node, setup, CONTAINER_FORGEJO_COMMANDS),
        data::NodeModel::HashicorpVault => vault_container(node, ztp, setup)?,
        data::NodeModel::KeycloakServer => keycloak_container(node, ztp, setup)?,
        model => tracing::debug!(%model, "Container keeps the manifest's settings"),
    }
    Ok(())
}

/// Generate the container's config files and mount them. Only the models whose config Sherpa
/// writes take `volume`.
fn container_volume_ztp(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    match node.model_kind.model() {
        data::NodeModel::AristaEos => arista_eos_container_volume_ztp(node, ztp, setup),
        data::NodeModel::NokiaSrlinux => srlinux_volume_ztp(node, ztp, setup),
        data::NodeModel::JuniperEvolved => cevolved_volume_ztp(node, node_image, ztp, setup),
        data::NodeModel::CiscoIol | data::NodeModel::CiscoIoll2 => {
            iol_volume_ztp(node, node_image, ztp, setup)
        }
        data::NodeModel::FrrLinux => frr_volume_ztp(node, ztp, setup),
        data::NodeModel::SonicLinux => sonic_container_volume_ztp(node, node_image, ztp, setup),
        model => bail!(
            "ZTP setting 'volume' of container node '{}' was not validated: Sherpa generates \
             no config for model {model}",
            node.name
        ),
    }
}

/// The manifest's mounts and the node certificate mounts, after the mounts its model's setup
/// added. The manifest's are dropped for a node whose config Sherpa mounts (`volume`), and for
/// Vault, GitLab CE and `SurrealDB`. A node with ZTP off keeps them: they are how its user
/// mounts a config of their own.
fn add_container_volumes(
    node: &topology::NodeExpanded,
    setting: data::Ztp,
    cert_paths: Option<&NodeCertPaths>,
    volumes: &mut Vec<String>,
) {
    let drops_manifest_volumes = setting == data::Ztp::Volume
        || matches!(
            node.model_kind.model(),
            data::NodeModel::GitlabCe
                | data::NodeModel::SurrealDb
                | data::NodeModel::HashicorpVault
        );
    if let Some(node_volumes) = node
        .container
        .as_ref()
        .and_then(|container| container.volumes.as_ref())
        && !drops_manifest_volumes
    {
        volumes.extend(node_volumes.iter().map(|v| format!("{}:{}", v.src, v.dst)));
    }
    // Add TLS certificate volume mounts if cert paths are provided
    if let Some(certs) = cert_paths {
        volumes.push(format!("{}:{}/ca.crt:ro", certs.ca_cert, NODE_CERTS_DIR));
        volumes.push(format!(
            "{}:{}/node.crt:ro",
            certs.node_cert, NODE_CERTS_DIR
        ));
        volumes.push(format!("{}:{}/node.key:ro", certs.node_key, NODE_CERTS_DIR));
    }
}

/// cEOS start settings: privileged, its environment and the init command.
fn arista_eos_container_start(node: &mut topology::NodeExpanded, setup: &mut ContainerSetup) {
    setup.privileged = true;
    setup.env_vars = CONTAINER_ARISTA_CEOS_ENV_VARS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    setup.commands = CONTAINER_ARISTA_CEOS_COMMANDS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    // Update node fields for consistency
    node.container.get_or_insert_default().privileged = Some(setup.privileged);
    node.environment_variables = Some(setup.env_vars.clone());
    node.container.get_or_insert_default().commands = Some(setup.commands.clone());
}

/// cEOS ZTP: its startup config, mounted read-only and copied into flash on first boot.
fn arista_eos_container_volume_ztp(
    node: &mut topology::NodeExpanded,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    let ContainerZtp {
        dir,
        custom_ztp,
        guest_account,
        dns,
        mgmt_net,
        node_ipv4_address,
        ..
    } = *ztp;
    let rendered_template = if let Some(config) = custom_ztp {
        config.to_owned()
    } else {
        template::AristaEosContainerZtpTemplate {
            hostname: node.name.clone(),
            owner: guest_account::owner_user(
                guest_account,
                KeyFormat::OpenSsh,
                PasswordForm::Hashed(CryptHash::Sha512),
            )?,
            dns: dns.clone(),
            mgmt_ipv4_address: Some(node_ipv4_address),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?
    };
    let ztp_config = format!("{dir}/{}.conf", node.name);
    let ztp_volume = topology::VolumeMount {
        src: ztp_config.clone(),
        dst: ARISTA_CEOS_ZTP_VOLUME_MOUNT.to_string(),
    };
    util::create_dir_with_permissions(Path::new(dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&ztp_config),
        rendered_template.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    setup.volumes = vec![format!("{}:{}:ro", ztp_volume.src, ztp_volume.dst)];
    setup.commands = ceos_startup_commands();

    node.container.get_or_insert_default().volumes = Some(vec![ztp_volume]);
    node.container.get_or_insert_default().commands = Some(setup.commands.clone());
    Ok(())
}

/// SR Linux start settings: privileged, its own user, environment and command.
fn srlinux_start(node: &mut topology::NodeExpanded, setup: &mut ContainerSetup) {
    setup.privileged = true;
    setup.user = Some(CONTAINER_NOKIA_SRLINUX_USER.to_string());
    setup.env_vars = CONTAINER_NOKIA_SRLINUX_ENV_VARS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    setup.commands = CONTAINER_NOKIA_SRLINUX_COMMANDS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    // Update node fields
    node.container.get_or_insert_default().privileged = Some(setup.privileged);
    node.container
        .get_or_insert_default()
        .user
        .clone_from(&setup.user);
    node.environment_variables = Some(setup.env_vars.clone());
    node.container.get_or_insert_default().commands = Some(setup.commands.clone());
}

/// SR Linux ZTP: its startup config as JSON, mounted over the image's.
fn srlinux_volume_ztp(
    node: &mut topology::NodeExpanded,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    let ContainerZtp {
        dir,
        custom_ztp,
        guest_account,
        dns,
        mgmt_net,
        node_ipv4_address,
        ..
    } = *ztp;
    let srlinux_config = match custom_ztp {
        Some(config) => config.to_owned(),
        None => template::build_srlinux_config(
            &node.name,
            &guest_account::owner_user(guest_account, KeyFormat::OpenSsh, PasswordForm::Plain)?,
            dns,
            &mgmt_net.v4,
            Some(node_ipv4_address),
            node.ipv6_address.zip(mgmt_net.v6.as_ref()),
        )?,
    };
    let ztp_config = format!("{dir}/{}.json", node.name);
    let ztp_volume = topology::VolumeMount {
        src: ztp_config.clone(),
        dst: NOKIA_SRLINUX_ZTP_VOLUME_MOUNT.to_string(),
    };
    util::create_dir_with_permissions(Path::new(dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&ztp_config),
        srlinux_config.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    setup.volumes = vec![format!("{}:{}", ztp_volume.src, ztp_volume.dst)];
    node.container.get_or_insert_default().volumes = Some(vec![ztp_volume]);
    Ok(())
}

/// cJunosEvolved start settings: privileged, and the environment its entrypoint
/// reads to start the EVO VM.
fn cevolved_start(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    setup: &mut ContainerSetup,
) -> Result<()> {
    // The entrypoint waits for this many data interfaces before it starts the EVO VM.
    let data_interfaces =
        effective_data_interface_count(&node.name, node.interfaces.data_count, node_image)?;
    setup.privileged = true;
    setup.env_vars = cevolved_env_vars(
        node.container
            .as_ref()
            .and_then(|container| container.platform)
            .unwrap_or_default(),
        data_interfaces,
        node.cpu.count.unwrap_or(node_image.cpu.count),
        node.memory.unwrap_or(node_image.memory),
    );

    // Update node fields for consistency
    node.container.get_or_insert_default().privileged = Some(setup.privileged);
    node.environment_variables = Some(setup.env_vars.clone());
    Ok(())
}

/// cJunosEvolved ZTP: its Junos config in a mounted config directory.
fn cevolved_volume_ztp(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    let ContainerZtp {
        dir,
        custom_ztp,
        guest_account,
        mgmt_net,
        node_ipv4_address,
        ..
    } = *ztp;
    let rendered_template = if let Some(config) = custom_ztp {
        config.to_owned()
    } else {
        template::JunipervJunosZtpTemplate {
            hostname: node.name.clone(),
            owner: guest_account::owner_user(
                guest_account,
                KeyFormat::Junos,
                PasswordForm::Hashed(CryptHash::Sha512),
            )?,
            mgmt_interface: node_image.interfaces.management.to_string(),
            mgmt_ipv4_address: Some(node_ipv4_address),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?
    };
    // The entrypoint reads `juniper.conf` from its config directory, so the directory
    // is mounted.
    let config_dir = format!("{dir}/configdisk");
    let config_volume = topology::VolumeMount {
        src: config_dir.clone(),
        dst: JUNIPER_CEVOLVED_CONFIG_DIR.to_string(),
    };
    util::create_dir_with_permissions(Path::new(&config_dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&format!("{config_dir}/{JUNIPER_ZTP_CONFIG}")),
        rendered_template.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    setup.volumes = vec![format!("{}:{}:ro", config_volume.src, config_volume.dst)];
    node.container.get_or_insert_default().volumes = Some(vec![config_volume]);
    Ok(())
}

/// IOL start settings: not privileged, `NET_ADMIN`, the launcher's settings in the environment
/// the image's entrypoint writes its config from, and IOL's NVRAM directory in the node's
/// folder, so a config saved with `write memory` survives a cold boot. A redeploy makes the
/// folder again, which clears it.
fn iol_start(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    let data_interfaces =
        effective_data_interface_count(&node.name, node.interfaces.data_count, node_image)?;
    setup.env_vars = vec![
        format!("IOL_ID={}", iol_app_id(&node.name, node.index)?),
        format!("IOL_MEMORY_MB={}", node.memory.unwrap_or(node_image.memory)),
        format!("IOL_ETH_PORTS={}", u16::from(data_interfaces) + 1),
    ];
    setup.capabilities = model_capabilities(node.model_kind);

    let run_dir = format!("{}/run", ztp.dir);
    util::create_dir_with_permissions(Path::new(&run_dir), NODE_CONFIG_DIR_MODE)?;
    setup.volumes.push(format!("{run_dir}:{CISCO_IOL_RUN_DIR}"));

    node.environment_variables = Some(setup.env_vars.clone());
    Ok(())
}

/// The IOL application ID of the node `node_name`, the `index`th of its lab: its index, unique
/// in the lab. IOL derives its MAC addresses from the ID, so two IOL nodes with the same one
/// cannot talk.
pub(crate) fn iol_app_id(node_name: &str, index: u16) -> Result<u16> {
    if (1..=CISCO_IOL_MAX_APP_ID).contains(&index) {
        Ok(index)
    } else {
        bail!(
            "IOL node '{node_name}' is node {index} of its lab; an IOL node must be one of the \
             first {CISCO_IOL_MAX_APP_ID}, as its application ID is its place in the lab"
        )
    }
}

/// IOL ZTP: its startup config in a mounted config directory, which the launcher copies into
/// IOL's NVRAM directory at each start. A config saved in NVRAM wins over it.
fn iol_volume_ztp(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    let ContainerZtp {
        dir,
        custom_ztp,
        guest_account,
        dns,
        mgmt_net,
        node_ipv4_address,
        ..
    } = *ztp;
    let rendered_template = if let Some(config) = custom_ztp {
        config.to_owned()
    } else {
        template::CiscoIolZtpTemplate {
            hostname: node.name.clone(),
            owner: guest_account::owner_user(
                guest_account,
                KeyFormat::CiscoMd5,
                PasswordForm::Plain,
            )?,
            mgmt_interface: node_image.interfaces.management.to_string(),
            dns: dns.clone(),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv4_address: node_ipv4_address,
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
            switch: node.model_kind.model() == data::NodeModel::CiscoIoll2,
        }
        .render()?
    };
    let config_dir = format!("{dir}/config");
    let config_volume = topology::VolumeMount {
        src: config_dir.clone(),
        dst: CISCO_IOL_CONFIG_DIR.to_string(),
    };
    util::create_dir_with_permissions(Path::new(&config_dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&format!("{config_dir}/{CISCO_IOL_STARTUP_CONFIG}")),
        rendered_template.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    setup
        .volumes
        .push(format!("{}:{}:ro", config_volume.src, config_volume.dst));
    let run_volume = topology::VolumeMount {
        src: format!("{dir}/run"),
        dst: CISCO_IOL_RUN_DIR.to_string(),
    };
    node.container.get_or_insert_default().volumes = Some(vec![run_volume, config_volume]);
    Ok(())
}

/// FRR start settings: privileged and its environment. The image's own entrypoint
/// and `daemons` file run it.
fn frr_start(node: &mut topology::NodeExpanded, setup: &mut ContainerSetup) {
    setup.privileged = true;
    setup.env_vars = CONTAINER_FRR_ENV_VARS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    node.container.get_or_insert_default().privileged = Some(setup.privileged);
    node.environment_variables = Some(setup.env_vars.clone());
}

/// FRR ZTP: its daemons file, its config, and a startup script that adds the SSH server and
/// the logins.
fn frr_volume_ztp(
    node: &mut topology::NodeExpanded,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    let ContainerZtp {
        dir,
        custom_ztp,
        guest_account,
        mgmt_net,
        node_ipv4_address,
        ..
    } = *ztp;
    let rendered_daemons = template::FrrDaemonsTemplate {}.render()?;
    let daemons_file = format!("{dir}/daemons");

    let rendered_config = if let Some(config) = custom_ztp {
        config.to_owned()
    } else {
        template::FrrZtpTemplate {
            hostname: node.name.clone(),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv4_address: Some(node_ipv4_address),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?
    };
    let config_file = format!("{dir}/frr.conf");

    let rendered_startup = template::FrrStartupTemplate {
        hostname: node.name.clone(),
        owner: guest_account::owner_user(
            guest_account,
            KeyFormat::OpenSsh,
            PasswordForm::Hashed(CryptHash::Sha512),
        )?,
    }
    .render()?;
    let startup_file = format!("{dir}/sherpa-init.sh");

    util::create_dir_with_permissions(Path::new(dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&daemons_file),
        rendered_daemons.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;
    util::write_file_with_permissions(
        Path::new(&config_file),
        rendered_config.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;
    util::write_file_with_permissions(
        Path::new(&startup_file),
        rendered_startup.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    let daemons_volume = topology::VolumeMount {
        src: daemons_file,
        dst: FRR_ZTP_DAEMONS_MOUNT.to_string(),
    };
    let config_volume = topology::VolumeMount {
        src: config_file,
        dst: FRR_ZTP_CONFIG_MOUNT.to_string(),
    };
    let startup_volume = topology::VolumeMount {
        src: startup_file,
        dst: FRR_ZTP_STARTUP_MOUNT.to_string(),
    };

    setup.volumes = vec![
        format!("{}:{}", daemons_volume.src, daemons_volume.dst),
        format!("{}:{}", config_volume.src, config_volume.dst),
        format!("{}:{}", startup_volume.src, startup_volume.dst),
    ];
    setup.commands = vec!["/bin/sh".to_string(), FRR_ZTP_STARTUP_MOUNT.to_string()];

    node.container.get_or_insert_default().volumes =
        Some(vec![daemons_volume, config_volume, startup_volume]);
    node.container.get_or_insert_default().commands = Some(setup.commands.clone());
    Ok(())
}

/// `sonic_linux` container start settings: privileged, and Sherpa's `supervisord` config and start
/// script, mounted whatever the ZTP setting, so the image always waits for the data interfaces
/// before it maps them to ports. The script has no account and no config here; `volume` writes
/// it again with them.
fn sonic_container_start(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    let dir = ztp.dir;
    let supervisord = template::SonicLinuxContainerSupervisordTemplate {
        init_script: SONIC_CONTAINER_INIT_MOUNT.to_owned(),
    }
    .render()?;
    let supervisord_file = format!("{dir}/supervisord.conf");
    util::create_dir_with_permissions(Path::new(dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&supervisord_file),
        supervisord.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;
    let init_file = write_sonic_container_init(node, node_image, dir, None, None)?;

    setup.privileged = true;
    // Mounted with the manifest's, which a node with ZTP off keeps, so the node's own mounts
    // stay as the manifest has them.
    setup.volumes.extend([
        format!("{supervisord_file}:{SONIC_CONTAINER_SUPERVISORD_MOUNT}:ro"),
        format!("{init_file}:{SONIC_CONTAINER_INIT_MOUNT}:ro"),
    ]);
    setup.commands = CONTAINER_SONIC_COMMANDS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    let container = node.container.get_or_insert_default();
    container.privileged = Some(setup.privileged);
    container.commands = Some(setup.commands.clone());
    Ok(())
}

/// `sonic_linux` container ZTP: its `config_db.json`, which the start script loads on the first start,
/// and the start script again with the owner's account. A custom config replaces the
/// `config_db.json` only; the account is in the start script, so the owner still gets it.
fn sonic_container_volume_ztp(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    let ContainerZtp {
        dir,
        custom_ztp,
        guest_account,
        mgmt_net,
        ..
    } = *ztp;
    // Docker addresses `eth0`, so the config has no management entries.
    let config_db = match custom_ztp {
        Some(config) => config.to_owned(),
        None => template::SonicLinuxZtp {
            hostname: node.name.clone(),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv4_address: None,
            mgmt_ipv6_address: None,
            mgmt_ipv6: None,
        }
        .config(),
    };
    let config_file = format!("{dir}/config_db.json");
    util::create_dir_with_permissions(Path::new(dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&config_file),
        config_db.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;
    let owner = guest_account::owner_user(
        guest_account,
        KeyFormat::OpenSsh,
        PasswordForm::Hashed(CryptHash::Sha512),
    )?;
    write_sonic_container_init(
        node,
        node_image,
        dir,
        Some(owner),
        Some(SONIC_CONTAINER_CONFIG_DB_MOUNT),
    )?;

    setup.volumes.push(format!(
        "{config_file}:{SONIC_CONTAINER_CONFIG_DB_MOUNT}:ro"
    ));
    // `volume` drops the manifest's mounts, so the node's are Sherpa's three.
    node.container.get_or_insert_default().volumes = Some(vec![
        topology::VolumeMount {
            src: format!("{dir}/supervisord.conf"),
            dst: SONIC_CONTAINER_SUPERVISORD_MOUNT.to_owned(),
        },
        topology::VolumeMount {
            src: format!("{dir}/sherpa-init.sh"),
            dst: SONIC_CONTAINER_INIT_MOUNT.to_owned(),
        },
        topology::VolumeMount {
            src: config_file,
            dst: SONIC_CONTAINER_CONFIG_DB_MOUNT.to_owned(),
        },
    ]);
    Ok(())
}

/// Render a `sonic_linux` container's start script into `dir` and return its path. It waits for the
/// node's data interfaces, as many as the node has.
fn write_sonic_container_init(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    dir: &str,
    owner: Option<data::OwnerUser>,
    config_db: Option<&str>,
) -> Result<String> {
    let init = template::SonicLinuxContainerInitTemplate {
        data_count: effective_data_interface_count(
            &node.name,
            node.interfaces.data_count,
            node_image,
        )?,
        owner,
        config_db: config_db.map(str::to_owned),
    }
    .render()?;
    let init_file = format!("{dir}/sherpa-init.sh");
    util::write_file_with_permissions(
        Path::new(&init_file),
        init.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;
    Ok(init_file)
}

/// An application container: its model's command.
fn app_container(node: &mut topology::NodeExpanded, setup: &mut ContainerSetup, commands: &[&str]) {
    setup.commands = commands
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    node.container.get_or_insert_default().commands = Some(setup.commands.clone());
}

/// Vault: its config, serving the API over TLS with the lab certificate unless the config is
/// the node's own.
fn vault_container(
    node: &mut topology::NodeExpanded,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    let ContainerZtp {
        dir,
        custom_ztp,
        node_ipv4_address,
        cert_paths,
        ..
    } = *ztp;
    let rendered_config = if let Some(config) = custom_ztp {
        config.to_owned()
    } else {
        template::VaultConfigTemplate {
            node_name: node.name.clone(),
            node_address: node_ipv4_address.to_string(),
            api_port: VAULT_API_PORT,
            cluster_port: VAULT_CLUSTER_PORT,
            certs_dir: NODE_CERTS_DIR.to_owned(),
        }
        .render()?
    };
    let config_file = format!("{dir}/config.hcl");
    let config_volume = topology::VolumeMount {
        src: config_file.clone(),
        dst: VAULT_ZTP_CONFIG_MOUNT.to_string(),
    };
    util::create_dir_with_permissions(Path::new(dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&config_file),
        rendered_config.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    setup.env_vars = if custom_ztp.is_some() {
        CONTAINER_HASHICORP_VAULT_ENV_VARS
            .iter()
            .map(std::string::ToString::to_string)
            .collect()
    } else {
        // The generated config serves the API over TLS with the lab certificate.
        if cert_paths.is_none() {
            bail!(
                "Vault node '{}' needs a lab certificate to serve its API over TLS",
                node.name
            );
        }
        vault_env_vars(node_ipv4_address)
    };
    setup.volumes = vec![format!("{}:{}", config_volume.src, config_volume.dst)];
    setup.commands = CONTAINER_HASHICORP_VAULT_COMMANDS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    setup.capabilities = model_capabilities(node.model_kind);

    node.environment_variables = Some(setup.env_vars.clone());
    node.container.get_or_insert_default().volumes = Some(vec![config_volume]);
    node.container.get_or_insert_default().commands = Some(setup.commands.clone());
    Ok(())
}

/// Keycloak: HTTPS only, with the lab certificate, run as root, and with the node's own admin,
/// saved in the lab folder with the realm's users before the node starts.
fn keycloak_container(
    node: &mut topology::NodeExpanded,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    if ztp.cert_paths.is_none() {
        bail!(
            "Keycloak node '{}' needs a lab certificate to serve HTTPS",
            node.name
        );
    }
    let admin = ensure_keycloak_credentials(
        &Path::new(ztp.lab_dir).join(KEYCLOAK_DIR),
        &node.name,
        &keycloak_address(ztp.node_ipv4_address),
        ztp.owner,
    )?;
    setup.user = Some(CONTAINER_KEYCLOAK_USER.to_string());
    setup.env_vars = merge_env_vars(
        keycloak_env_vars(ztp.node_ipv4_address, &admin),
        node.environment_variables.as_deref().unwrap_or_default(),
    );
    setup.commands = CONTAINER_KEYCLOAK_COMMANDS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    node.environment_variables = Some(setup.env_vars.clone());
    node.container.get_or_insert_default().commands = Some(setup.commands.clone());
    node.container
        .get_or_insert_default()
        .user
        .clone_from(&setup.user);
    Ok(())
}

/// `SurrealDB`: in memory, with the node's own root user, saved in the lab folder before the
/// node starts.
fn surreal_db_container(
    node: &mut topology::NodeExpanded,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    let admin = ensure_surreal_db_credentials(
        &Path::new(ztp.lab_dir).join(SURREAL_DB_DIR),
        &node.name,
        &surreal_db_address(ztp.node_ipv4_address),
    )?;
    setup.env_vars = merge_env_vars(
        surreal_db_env_vars(&admin),
        node.environment_variables.as_deref().unwrap_or_default(),
    );
    node.environment_variables = Some(setup.env_vars.clone());
    app_container(node, setup, CONTAINER_SURREAL_DB_COMMANDS);
    Ok(())
}

/// How a P2p container's data interface veth comes up: a disabled one stays down, unless the
/// model keeps every data interface up, when only its host end is down.
pub(crate) fn netns_veth_state(model_kind: data::ModelKind, admin_down: bool) -> NetnsVethState {
    if !admin_down {
        NetnsVethState::Enabled
    } else if model_kind.keeps_data_interfaces_up() {
        NetnsVethState::DisabledUp
    } else {
        NetnsVethState::Disabled
    }
}

/// The environment of a cJunosEvolved container: the platform it emulates, how many data
/// interfaces its entrypoint waits for, and its EVO VM's vCPUs and memory.
fn cevolved_env_vars(
    platform: data::PtxPlatform,
    data_interfaces: u8,
    cpu_count: u8,
    memory: u16,
) -> Vec<String> {
    vec![
        format!("CPTX_COSIM={}", platform.cosim()),
        format!("CPTX_NUM_IF={data_interfaces}"),
        format!("CPTX_EVOVM_CPU={cpu_count}"),
        format!("CPTX_EVOVM_MEM_MB={memory}"),
    ]
}

/// The command a cEOS container starts with: copy the ZTP config into flash unless a saved
/// config is already there, then hand over to init.
fn ceos_startup_commands() -> Vec<String> {
    let init = CONTAINER_ARISTA_CEOS_COMMANDS.join(" ");
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!(
            "[ -f {ARISTA_CEOS_STARTUP_CONFIG} ] || cp {ARISTA_CEOS_ZTP_VOLUME_MOUNT} \
             {ARISTA_CEOS_STARTUP_CONFIG}; exec {init}"
        ),
    ]
}

// ============================================================================
// VM ZTP Generation
// ============================================================================

/// Generate ZTP configuration for a VM node.
/// Returns ZTP record, clone disks, disk definitions, MAC address, and QEMU commands.
#[instrument(level = "debug", skip_all, fields(node_name = %node.name))]
pub(crate) fn generate_vm_ztp(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab: &LabZtp<'_>,
    node_ipv4_address: Ipv4Addr,
    mgmt_mac: Option<&str>,
    cert_paths: Option<&NodeCertPaths>,
) -> Result<VmZtpResult> {
    let LabZtp {
        lab_id,
        lab_dir,
        images_dir,
        dns,
        mgmt_net,
        progress,
        owner_account,
    } = *lab;
    let tftp_dir = format!("{lab_dir}/{ZTP_DIR}/{TFTP_DIR}");
    let node_account = guest_account::node_owner(owner_account, node, node_image)?;
    let guest_account = node_account.as_ref();
    let node_name_with_lab = format!("{}-{}", node.name, lab_id);

    // Reuse existing MAC address if provided, otherwise generate a new one
    let mac_address = mgmt_mac.map_or_else(
        || util::random_mac(KVM_OUI),
        std::string::ToString::to_string,
    );

    let boot_dir = vm_boot_dir(lab_dir, &node.name);
    let ztp = NodeZtp {
        lab_dir,
        boot_dir: &boot_dir,
        tftp_dir: &tftp_dir,
        images_dir,
        device_password: &owner_account.device_password,
        dns,
        mgmt_net,
        progress,
        node_ipv4_address,
        mac_address: &mac_address,
        node_name_with_lab: &node_name_with_lab,
        cert_paths,
        guest_account,
    };
    let setting = effective_ztp(node.ztp.method, node_image);
    let media = generate_vm_ztp_media(node, node_image, &ztp, setting)?;
    // The image's own CD-ROM, such as vEOS's Aboot, unless the ZTP method brings one.
    let vm = vm_settings(node_image)?;
    let image_cdrom = vm.cdrom.image.as_ref().map(|src_iso| {
        (
            format!(
                "{}/{}/{}/{}",
                images_dir, node_image.model, ARISTA_ABOOT_DIR, src_iso
            ),
            format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}.iso"),
        )
    });
    let media = VmZtpMedia {
        cdrom: media.cdrom.or(image_cdrom),
        ..media
    };
    let (clone_disks, disks) = vm_disks(node, node_image, images_dir, &node_name_with_lab, &media)?;
    let firmware = VmFirmware {
        qemu_commands: vm_qemu_commands(node.model_kind),
        ignition_config: media.ignition.map(|(_, dst)| dst),
    };

    let ztp_record = data::ZtpRecord {
        node_name: node.name.clone(),
        config_file: format!("{}.conf", node.name),
        ipv4_address: node_ipv4_address,
        ipv6_address: None,
        mac_address: mac_address.clone(),
        ztp: setting,
        ssh_port: node.model_kind.has_ssh().then_some(SSH_PORT),
    };

    Ok(VmZtpResult {
        ztp_record,
        clone_disks,
        disks,
        mac_address,
        firmware,
    })
}

/// Generate the VM's configuration by its ZTP setting, and return the media that carry it to
/// the VM. With ZTP off there is nothing to generate and no media. `volume` is a container's
/// method, and the validator refuses it for a VM before the node is planned, so reaching it
/// here means a setting skipped validation.
fn generate_vm_ztp_media(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
    setting: data::Ztp,
) -> Result<VmZtpMedia> {
    let NodeZtp {
        boot_dir,
        progress,
        node_name_with_lab,
        ..
    } = *ztp;
    let custom_ztp = take_custom_ztp_config(node, progress);
    let custom_ztp = custom_ztp.as_deref();
    let ztp_iso = (
        format!("{boot_dir}/{ZTP_ISO}"),
        format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}.iso"),
    );
    let mut media = VmZtpMedia::default();

    match setting {
        data::Ztp::Disabled => {
            let _ = progress.send_status(
                format!("ZTP is off for node: {}", node.name),
                StatusKind::Info,
            );
        }
        data::Ztp::CloudInit => {
            generate_cloud_init_ztp(node, node_image, ztp)?;
            media.cdrom = Some(ztp_iso);
        }
        data::Ztp::Tftp => generate_tftp_ztp(node, node_image, ztp, custom_ztp)?,
        data::Ztp::NoCloud => {
            generate_nocloud_ztp(node, ztp, custom_ztp)?;
            media.cdrom = Some(ztp_iso);
        }
        data::Ztp::Cdrom => {
            generate_cdrom_ztp(node, node_image, ztp, custom_ztp)?;
            media.cdrom = Some(ztp_iso);
        }
        data::Ztp::Disk => {
            media.config_disk = Some(generate_disk_ztp(node, node_image, ztp, custom_ztp)?);
        }
        data::Ztp::Usb => {
            media.usb_disk = Some(generate_usb_ztp(node, node_image, ztp, custom_ztp)?);
        }
        data::Ztp::Http => generate_http_ztp(node, node_image, ztp)?,
        data::Ztp::Ignition => {
            let (src_ign, dst_ign, src_cfg, dst_cfg) =
                generate_ignition_ztp(node, node_image, ztp)?;
            media.ignition = Some((src_ign, dst_ign));
            media.config_disk = Some((src_cfg, dst_cfg));
        }
        data::Ztp::Volume => bail!(
            "ZTP setting '{setting}' of virtual machine '{}' was not validated: it is a \
             container's method",
            node.name
        ),
    }
    Ok(media)
}

/// A disk copied as it is into the storage pool.
fn copied_disk(src: &str, dst: &str) -> data::CloneDisk {
    data::CloneDisk {
        src: src.to_owned(),
        dst: dst.to_owned(),
        disk_size: None,
        method: data::DiskImageCloneMethod::Copy,
    }
}

/// The VM's disks to clone into the storage pool, and its domain's disks: the CD-ROM, the boot
/// disk, then the config and USB disks.
fn vm_disks(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    images_dir: &str,
    node_name_with_lab: &str,
    media: &VmZtpMedia,
) -> Result<(Vec<data::CloneDisk>, Vec<data::NodeDisk>)> {
    let vm = vm_settings(node_image)?;
    let hdd_bus = &vm.boot_disk.bus;
    let cdrom_bus = &vm.cdrom.bus;
    let filename = VM_DISK_FILENAME;
    let dst_boot_disk = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-hdd.qcow2");
    let mut clone_disks = vec![data::CloneDisk {
        src: format!(
            "{}/{}/{}/{}",
            images_dir, node_image.model, node_image.version, filename
        ),
        dst: dst_boot_disk.clone(),
        disk_size: node.boot_disk.size,
        method: node.boot_disk.clone_method.unwrap_or_default(),
    }];
    let mut disks: Vec<data::NodeDisk> = vec![];

    if let Some((src, dst)) = &media.cdrom {
        clone_disks.push(copied_disk(src, dst));
        disks.push(data::NodeDisk {
            disk_device: data::DiskDevices::Cdrom,
            driver_name: data::DiskDrivers::Qemu,
            driver_format: data::DiskFormats::Raw,
            src_file: dst.clone(),
            target_dev: data::DiskTargets::target(cdrom_bus, disk_index(&disks)?)?,
            target_bus: cdrom_bus.clone(),
        });
    }

    disks.push(data::NodeDisk {
        disk_device: data::DiskDevices::File,
        driver_name: data::DiskDrivers::Qemu,
        driver_format: data::DiskFormats::Qcow2,
        src_file: dst_boot_disk,
        target_dev: data::DiskTargets::target(hdd_bus, disk_index(&disks)?)?,
        target_bus: hdd_bus.clone(),
    });

    if let Some((src, dst)) = &media.config_disk {
        clone_disks.push(copied_disk(src, dst));
        disks.push(data::NodeDisk {
            disk_device: data::DiskDevices::File,
            driver_name: data::DiskDrivers::Qemu,
            driver_format: data::DiskFormats::Raw,
            src_file: dst.clone(),
            target_dev: data::DiskTargets::target(hdd_bus, disk_index(&disks)?)?,
            target_bus: hdd_bus.clone(),
        });
    }

    if let Some((src, dst)) = &media.usb_disk {
        clone_disks.push(copied_disk(src, dst));
        // USB and SATA share the sd* target namespace, so count existing sd* targets
        // to avoid duplicate target device names.
        let usb_target_index = disks
            .iter()
            .filter(|d| matches!(d.target_bus, data::DiskBuses::Sata | data::DiskBuses::Usb))
            .count();
        disks.push(data::NodeDisk {
            disk_device: data::DiskDevices::File,
            driver_name: data::DiskDrivers::Qemu,
            driver_format: data::DiskFormats::Raw,
            src_file: dst.clone(),
            target_dev: data::DiskTargets::target(
                &data::DiskBuses::Usb,
                u8::try_from(usb_target_index).context("Too many disks for one node")?,
            )?,
            target_bus: data::DiskBuses::Usb,
        });
    }

    // The Ignition config is a firmware config entry of the domain, not a disk.
    if let Some((src, dst)) = &media.ignition {
        clone_disks.push(copied_disk(src, dst));
    }
    Ok((clone_disks, disks))
}

/// The extra QEMU arguments of a VM model.
fn vm_qemu_commands(model_kind: data::ModelKind) -> Vec<data::QemuCommand> {
    match model_kind.model() {
        data::NodeModel::JuniperRouter => data::QemuCommand::juniper_router(),
        data::NodeModel::JuniperSwitch => data::QemuCommand::juniper_switch(),
        data::NodeModel::JuniperSrxv3 => data::QemuCommand::juniper_srxv3(),
        data::NodeModel::JuniperEvolved => data::QemuCommand::juniper_evolved(),
        _ => vec![],
    }
}

// ============================================================================
// TLS Certificate Helpers
// ============================================================================

/// Sherpa's `KEY=value` variables for a node, with the user's added; a user variable replaces
/// Sherpa's of the same key.
fn merge_env_vars(defaults: Vec<String>, user: &[String]) -> Vec<String> {
    let mut merged = Vec::with_capacity(defaults.len() + user.len());
    for default in defaults {
        let key = default.split('=').next().unwrap_or_default();
        let overridden = user
            .iter()
            .any(|var| var.split('=').next().unwrap_or_default() == key);
        if !overridden {
            merged.push(default);
        }
    }
    merged.extend(user.iter().cloned());
    merged
}

/// The environment of a Keycloak node: production mode with its built-in file database, HTTPS
/// only on the node's address with the lab certificate, and `admin`, the node's own, as the
/// bootstrap admin of the `master` realm. `docker inspect` shows the admin's password to the
/// host's `docker` group, which is accepted: that group is root on the host.
fn keycloak_env_vars(node_address: Ipv4Addr, admin: &data::AdminCredential) -> Vec<String> {
    vec![
        "KC_DB=dev-file".to_owned(),
        "KC_HTTP_ENABLED=false".to_owned(),
        format!("KC_HOSTNAME=https://{node_address}:{KEYCLOAK_HTTPS_PORT}"),
        format!("KC_HTTPS_CERTIFICATE_FILE={NODE_CERTS_DIR}/node.crt"),
        format!("KC_HTTPS_CERTIFICATE_KEY_FILE={NODE_CERTS_DIR}/node.key"),
        "KC_HEALTH_ENABLED=true".to_owned(),
        format!("KC_BOOTSTRAP_ADMIN_USERNAME={}", admin.username),
        format!("KC_BOOTSTRAP_ADMIN_PASSWORD={}", admin.password),
    ]
}

/// The environment of a `SurrealDB` node: `admin`, the node's own, as its root user. As with
/// Keycloak, `docker inspect` shows the password to the host's `docker` group.
fn surreal_db_env_vars(admin: &data::AdminCredential) -> Vec<String> {
    vec![
        format!("SURREAL_USER={}", admin.username),
        format!("SURREAL_PASS={}", admin.password),
    ]
}

/// The environment of a Vault node with the generated config: the `vault` CLI reaches the API
/// over TLS on the node's address and trusts the lab CA.
fn vault_env_vars(node_address: Ipv4Addr) -> Vec<String> {
    vec![
        format!("VAULT_ADDR=https://{node_address}:{VAULT_API_PORT}"),
        format!("VAULT_CACERT={NODE_CERTS_DIR}/ca.crt"),
    ]
}

/// The lab CA for cloud-init to add to the node's trust store, when the node has a lab
/// certificate.
fn cloud_init_ca_certs(
    cert_paths: Option<&NodeCertPaths>,
) -> Result<Option<template::CloudInitCaCerts>> {
    let Some(certs) = cert_paths else {
        return Ok(None);
    };
    let ca_cert = std::fs::read_to_string(&certs.ca_cert)
        .with_context(|| format!("Failed to read CA cert: {}", certs.ca_cert))?;
    Ok(Some(template::CloudInitCaCerts {
        trusted: vec![ca_cert],
    }))
}

/// Build cloud-init `write_files` entries for TLS certificates
fn build_cloud_init_cert_files(
    cert_paths: &NodeCertPaths,
    target_dir: &str,
) -> Result<Vec<template::CloudInitWriteFile>> {
    let ca_cert_content = std::fs::read_to_string(&cert_paths.ca_cert)
        .with_context(|| format!("Failed to read CA cert: {}", cert_paths.ca_cert))?;
    let node_cert_content = std::fs::read_to_string(&cert_paths.node_cert)
        .with_context(|| format!("Failed to read node cert: {}", cert_paths.node_cert))?;
    let node_key_content = std::fs::read_to_string(&cert_paths.node_key)
        .with_context(|| format!("Failed to read node key: {}", cert_paths.node_key))?;

    Ok(vec![
        template::CloudInitWriteFile {
            path: format!("{target_dir}/ca.crt"),
            content: ca_cert_content,
            permissions: "0644".to_string(),
            owner: None,
            encoding: None,
        },
        template::CloudInitWriteFile {
            path: format!("{target_dir}/node.crt"),
            content: node_cert_content,
            permissions: "0644".to_string(),
            owner: None,
            encoding: None,
        },
        template::CloudInitWriteFile {
            path: format!("{target_dir}/node.key"),
            content: node_key_content,
            permissions: "0600".to_string(),
            owner: None,
            encoding: None,
        },
    ])
}

/// Build cloudbase-init `write_files` entries for TLS certificates (Windows)
fn build_cloudbase_cert_files(
    cert_paths: &NodeCertPaths,
    target_dir: &str,
) -> Result<Vec<template::CloudbaseWriteFile>> {
    let ca_cert_content = std::fs::read_to_string(&cert_paths.ca_cert)
        .with_context(|| format!("Failed to read CA cert: {}", cert_paths.ca_cert))?;
    let node_cert_content = std::fs::read_to_string(&cert_paths.node_cert)
        .with_context(|| format!("Failed to read node cert: {}", cert_paths.node_cert))?;
    let node_key_content = std::fs::read_to_string(&cert_paths.node_key)
        .with_context(|| format!("Failed to read node key: {}", cert_paths.node_key))?;

    Ok(vec![
        template::CloudbaseWriteFile {
            path: format!(r"{target_dir}\ca.crt"),
            content: ca_cert_content,
            permissions: "0644".to_string(),
        },
        template::CloudbaseWriteFile {
            path: format!(r"{target_dir}\node.crt"),
            content: node_cert_content,
            permissions: "0644".to_string(),
        },
        template::CloudbaseWriteFile {
            path: format!(r"{target_dir}\node.key"),
            content: node_key_content,
            permissions: "0600".to_string(),
        },
    ])
}

/// Build staged files from pre-encoded `text_files`
fn build_cloud_init_text_files(text_files: Option<&[topology::TextFileData]>) -> Vec<StagedFile> {
    text_files
        .into_iter()
        .flatten()
        .map(|file| StagedFile {
            dst: file.dst.clone(),
            content: file.content.clone(),
            encoding: Some("b64".to_string()),
            user: file.user.clone(),
            group: Some(file.group.clone()),
            permissions: file.permissions.to_string(),
        })
        .collect()
}

/// Build cloudbase-init `write_files` entries from pre-encoded `text_files`
fn build_cloudbase_text_files(
    text_files: Option<&[topology::TextFileData]>,
) -> Vec<template::CloudbaseWriteFile> {
    text_files
        .into_iter()
        .flatten()
        .map(|file| template::CloudbaseWriteFile {
            path: file.dst.clone(),
            content: file.content.clone(),
            permissions: format!("0{}", file.permissions),
        })
        .collect()
}

/// Build Ignition file entries for TLS certificates
fn build_ignition_cert_files(
    cert_paths: &NodeCertPaths,
    target_dir: &str,
) -> Result<Vec<template::IgnitionFile>> {
    let ca_cert_content = std::fs::read_to_string(&cert_paths.ca_cert)
        .with_context(|| format!("Failed to read CA cert: {}", cert_paths.ca_cert))?;
    let node_cert_content = std::fs::read_to_string(&cert_paths.node_cert)
        .with_context(|| format!("Failed to read node cert: {}", cert_paths.node_cert))?;
    let node_key_content = std::fs::read_to_string(&cert_paths.node_key)
        .with_context(|| format!("Failed to read node key: {}", cert_paths.node_key))?;

    let ca_b64 = util::base64_encode(&ca_cert_content);
    let cert_b64 = util::base64_encode(&node_cert_content);
    let key_b64 = util::base64_encode(&node_key_content);

    Ok(vec![
        template::IgnitionFile {
            path: format!("{target_dir}/ca.crt"),
            mode: template::FileMode::new(0o644),
            contents: Some(template::IgnitionFileContents::new(&format!(
                "data:;base64,{ca_b64}"
            ))),
            ..Default::default()
        },
        template::IgnitionFile {
            path: format!("{target_dir}/node.crt"),
            mode: template::FileMode::new(0o644),
            contents: Some(template::IgnitionFileContents::new(&format!(
                "data:;base64,{cert_b64}"
            ))),
            ..Default::default()
        },
        template::IgnitionFile {
            path: format!("{target_dir}/node.key"),
            mode: template::FileMode::new(0o600),
            contents: Some(template::IgnitionFileContents::new(&format!(
                "data:;base64,{key_b64}"
            ))),
            ..Default::default()
        },
    ])
}

// ============================================================================
// Environment variable helpers
// ============================================================================

/// Build a shell script that exports environment variables when sourced.
fn build_env_profile_script(env_vars: &[String]) -> String {
    let mut script = "#!/bin/sh\n".to_string();
    for entry in env_vars {
        if let Some((key, value)) = entry.split_once('=') {
            // Escape single quotes: ' → '\''
            let escaped_value = value.replace('\'', "'\\''");
            script.extend(["export ", key, "='", &escaped_value, "'\n"]);
        }
    }
    script
}

/// Line added to a login file to source the user's environment file.
fn user_env_loader() -> String {
    format!(r#"if [ -r "$HOME/{NODE_USER_ENV_FILE}" ]; then . "$HOME/{NODE_USER_ENV_FILE}"; fi"#)
}

/// Build the guest login's private environment file.
fn build_user_env_file(env_vars: &[String], username: &str) -> StagedFile {
    StagedFile {
        dst: format!("~/{NODE_USER_ENV_FILE}"),
        content: build_env_profile_script(env_vars),
        encoding: None,
        user: username.to_owned(),
        group: None,
        permissions: "600".to_string(),
    }
}

/// Shell snippet that sets `h` to the home directory of the user in `u`, from `passwd_file`.
fn passwd_home_lookup(passwd_file: &str) -> String {
    format!(
        r#"h=$(awk -F: -v u="$u" '$1 == u {{ print $6 }}' {})"#,
        shell_quote(passwd_file)
    )
}

/// Build the runcmd that sources the environment file from the login file bash reads:
/// `~/.bash_profile` when present, otherwise `~/.profile`. The home directory comes from
/// `passwd_file`, and nothing is done when the user does not exist.
fn user_env_loader_runcmd(username: &str, passwd_file: &str) -> String {
    format!(
        r#"u={}; {}; [ -n "$h" ] && {{ f="$h/.bash_profile"; [ -f "$f" ] || f="$h/.profile"; printf '%s\n' '{}' >> "$f"; chown "$u:$(id -gn "$u")" "$f"; }}"#,
        shell_quote(username),
        passwd_home_lookup(passwd_file),
        user_env_loader()
    )
}

/// Quote a value as one POSIX shell word.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// cloud-init files and runcmd lines for a node's `startup_scripts`, run as root. Each script
/// is run directly, so its shebang picks the interpreter and nodes without `bash` can run them.
fn build_cloud_init_scripts(
    node_name: &str,
    startup_scripts: Option<&[topology::StartupScript]>,
) -> Result<(Vec<template::CloudInitWriteFile>, Vec<String>)> {
    let mut write_files = vec![];
    let mut runcmd = vec![];

    for script in startup_scripts.unwrap_or_default() {
        let contents = util::base64_decode(&script.content).with_context(|| {
            format!(
                "Failed to decode startup_script '{}' for node '{}'",
                script.filename, node_name
            )
        })?;
        let target_path = format!("{}/{}", NODE_STARTUP_SCRIPTS_DIR, script.filename);
        write_files.push(template::CloudInitWriteFile {
            path: target_path.clone(),
            content: contents,
            permissions: "0755".to_string(),
            owner: None,
            encoding: None,
        });
        runcmd.push(shell_quote(&target_path));
    }

    Ok((write_files, runcmd))
}

/// Write staged files as root-only files in `staging_dir`, and build the runcmd that moves
/// each into place. Missing parent directories inside the user's home are created owned by
/// the user, and others stay root-owned. Files for users missing from `passwd_file` are
/// skipped. The staging directory is removed once every file has been handled.
fn stage_cloud_init_files(
    files: &[StagedFile],
    staging_dir: &str,
    passwd_file: &str,
) -> (Vec<template::CloudInitWriteFile>, Vec<String>) {
    let mut writes = Vec::new();
    let mut runcmd = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let src = format!("{staging_dir}/{index}");
        let group = match &file.group {
            Some(group) => shell_quote(group),
            None => r#""$(id -gn "$u")""#.to_string(),
        };
        let dst = match file.dst.strip_prefix("~/") {
            Some(relative) => format!(r#""$h"/{}"#, shell_quote(relative)),
            None => shell_quote(&file.dst),
        };
        writes.push(template::CloudInitWriteFile {
            path: src.clone(),
            content: file.content.clone(),
            permissions: "0600".to_string(),
            owner: None,
            encoding: file.encoding.clone(),
        });
        runcmd.push(format!(
            r#"u={}; {}; g={group}; src={}; [ -n "$h" ] && dst={dst} && dir=$(dirname "$dst") && p=$dir && top= && while [ ! -d "$p" ]; do top=$p; p=$(dirname "$p"); done && mkdir -p "$dir" && {{ [ -z "$top" ] || case "$top" in "$h"|"$h"/*) chown -R "$u:$g" "$top";; esac; }} && mv "$src" "$dst" && chown "$u:$g" "$dst" && chmod {} "$dst""#,
            shell_quote(&file.user),
            passwd_home_lookup(passwd_file),
            shell_quote(&src),
            file.permissions,
        ));
    }
    if !files.is_empty() {
        runcmd.push(format!("rm -rf {}", shell_quote(staging_dir)));
    }
    (writes, runcmd)
}

/// Build Ignition directories for the missing parents of `text_files` under their owner's
/// home, owned by that user. Ignition otherwise creates them as root, which leaves the user
/// unable to write inside, for example, their own `~/.config`.
fn build_ignition_home_directories(
    text_files: Option<&[topology::TextFileData]>,
) -> Vec<template::IgnitionDirectory> {
    let mut directories: Vec<template::IgnitionDirectory> = Vec::new();
    for file in text_files.into_iter().flatten() {
        if file.user == "root" {
            continue;
        }
        let home = format!("/home/{}", file.user);
        let Some(parent) = Path::new(&file.dst).parent() else {
            continue;
        };
        let mut parents = Vec::new();
        for ancestor in parent.ancestors() {
            if !ancestor.starts_with(&home) || ancestor == Path::new(&home) {
                break;
            }
            parents.push(ancestor.to_string_lossy().into_owned());
        }
        for path in parents.into_iter().rev() {
            let mut seen = false;
            for directory in &directories {
                if directory.path == path {
                    seen = true;
                }
            }
            if seen {
                continue;
            }
            directories.push(template::IgnitionDirectory {
                path,
                mode: template::FileMode::new(0o755),
                overwrite: false,
                user: Some(template::IgnitionFileParams {
                    name: file.user.clone(),
                }),
                group: Some(template::IgnitionFileParams {
                    name: file.group.clone(),
                }),
            });
        }
    }
    directories
}

/// Build Ignition files for the guest login's private environment file, and append its
/// loader to `~/.bash_profile`, which Flatcar copies from the skeleton for every user.
fn build_ignition_user_env(env_vars: &[String], username: &str) -> Vec<template::IgnitionFile> {
    let home = format!("/home/{username}");
    let env_b64 = util::base64_encode(&build_env_profile_script(env_vars));
    let loader_b64 = util::base64_encode(&format!("\n{}\n", user_env_loader()));
    vec![
        template::IgnitionFile {
            path: format!("{home}/{NODE_USER_ENV_FILE}"),
            mode: template::FileMode::new(0o600),
            overwrite: Some(true),
            contents: Some(template::IgnitionFileContents::new(&format!(
                "data:;base64,{env_b64}"
            ))),
            user: Some(template::IgnitionFileParams {
                name: username.to_owned(),
            }),
            group: Some(template::IgnitionFileParams {
                name: username.to_owned(),
            }),
            ..Default::default()
        },
        template::IgnitionFile {
            path: format!("{home}/.bash_profile"),
            mode: template::FileMode::new(0o644),
            append: vec![template::IgnitionFileContents::new(&format!(
                "data:;base64,{loader_b64}"
            ))],
            user: Some(template::IgnitionFileParams {
                name: username.to_owned(),
            }),
            group: Some(template::IgnitionFileParams {
                name: username.to_owned(),
            }),
            ..Default::default()
        },
    ]
}

/// Build the commands that set each of `env_vars` as a machine-level environment variable on
/// Windows.
///
/// cloudbase-init runs each command through `cmd`, so the PowerShell command is passed with
/// `-EncodedCommand`, as base64 of its UTF-16LE bytes: `cmd` never sees the key or the value,
/// so a `"` in them cannot end an argument and a `%NAME%` is not expanded. Inside the
/// PowerShell command the key and value are single-quoted strings, with `'` written `''`.
fn build_windows_env_commands(env_vars: &[String]) -> Vec<String> {
    let mut commands = Vec::new();
    for entry in env_vars {
        if let Some((key, value)) = entry.split_once('=') {
            let command = format!(
                "[System.Environment]::SetEnvironmentVariable('{}', '{}', 'Machine')",
                key.replace('\'', "''"),
                value.replace('\'', "''")
            );
            let utf16le: Vec<u8> = command.encode_utf16().flat_map(u16::to_le_bytes).collect();
            commands.push(format!(
                "powershell -NoProfile -EncodedCommand {}",
                util::base64_encode_bytes(&utf16le)
            ));
        }
    }
    commands
}

/// Build `icacls` commands that make Windows files private.
///
/// cloudbase-init ignores `permissions`, so a written file inherits its folder's ACL, which
/// lets every local user read it. A file whose mode gives others no access (such as a node
/// key at `0600`) is limited to Administrators and SYSTEM instead, named by SID so the
/// command does not depend on the Windows display language. The commands run from cmd, so
/// paths with `"` or `%` are refused rather than quoted.
fn build_windows_private_file_acls(
    write_files: &[template::CloudbaseWriteFile],
) -> Result<Vec<String>> {
    let mut commands = Vec::new();
    for file in write_files {
        if !file.permissions.ends_with('0') {
            continue;
        }
        if file.path.contains(['"', '%']) {
            return Err(ServiceError::Invalid(format!(
                r#"Windows file path '{}' contains '"' or '%', which cannot be used in a path on a Windows node"#,
                file.path
            ))
            .into());
        }
        commands.push(format!(
            r#"icacls "{}" /inheritance:r /grant {WINDOWS_ADMINISTRATORS_SID}:F /grant {WINDOWS_SYSTEM_SID}:F"#,
            file.path
        ));
    }
    Ok(commands)
}

// ============================================================================
// VM ZTP sub-methods
// ============================================================================

/// The IPv6 address to put in a node's cloud-init `network-config`.
///
/// OpenBSD gets none: cloud-init's OpenBSD renderer never writes IPv6 addresses, and it
/// crashes on the IPv6 default route, which leaves the node with no network at all.
fn cloud_init_ipv6_address(
    model_kind: data::ModelKind,
    address: Option<Ipv6Addr>,
) -> Option<Ipv6Addr> {
    match model_kind.model() {
        data::NodeModel::OpenBsd => None,
        _ => address,
    }
}

/// Whether cloud-init should manage a node's `/etc/hosts`.
///
/// OpenBSD does not: cloud-init ships no hosts template for it, so the module fails and
/// cloud-init reports an error. The node's name resolves through the lab DNS instead.
fn cloud_init_manage_etc_hosts(model_kind: data::ModelKind) -> bool {
    !matches!(model_kind.model(), data::NodeModel::OpenBsd)
}

/// The doas rule giving `wheel` admin rights on OpenBSD, where doas is the native tool: the
/// stock `/etc/examples/doas.conf` rule, without a password, as the owner keeps passwordless
/// admin rights on every node. The image has no `/etc/doas.conf`.
fn cloud_init_doas_file(model_kind: data::ModelKind) -> Option<template::CloudInitWriteFile> {
    if model_kind.model() != data::NodeModel::OpenBsd {
        return None;
    }
    Some(template::CloudInitWriteFile {
        path: NODE_DOAS_CONF_FILE.to_owned(),
        content: "permit nopass keepenv :wheel\n".to_owned(),
        permissions: "0400".to_owned(),
        owner: None,
        encoding: None,
    })
}

/// The cloud-init `sudo` rule for admin logins, except on OpenBSD, which uses doas, and NixOS,
/// whose generated `/etc/sudoers` does not read `/etc/sudoers.d`; there `wheel` grants sudo.
fn cloud_init_sudo_rule(model_kind: data::ModelKind) -> Option<String> {
    match model_kind.model() {
        data::NodeModel::OpenBsd | data::NodeModel::NixosLinux => None,
        _ => Some(NODE_CLOUD_INIT_SUDO_RULE.to_owned()),
    }
}

/// Whether cloud-init allows SSH password logins, or `None` to leave the SSH server alone. NixOS
/// generates `sshd_config`, which cloud-init cannot edit, and its image allows password logins
/// itself.
fn cloud_init_ssh_pwauth(model_kind: data::ModelKind) -> Option<bool> {
    match model_kind.model() {
        data::NodeModel::NixosLinux => None,
        _ => Some(true),
    }
}

#[instrument(level = "debug", skip_all, fields(node_name = %node.name))]
fn generate_cloud_init_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
) -> Result<()> {
    let _ = ztp.progress.send_status(
        format!("Creating Cloud-Init config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let dir = ztp.boot_dir.to_owned();
    match node.model_kind.model() {
        data::NodeModel::CentosLinux
        | data::NodeModel::AlmaLinux
        | data::NodeModel::DevboxLinux
        | data::NodeModel::RockyLinux
        | data::NodeModel::FedoraLinux
        | data::NodeModel::OpensuseLinux
        | data::NodeModel::RedhatLinux
        | data::NodeModel::SuseLinux
        | data::NodeModel::UbuntuLinux
        | data::NodeModel::OmarchyLinux
        | data::NodeModel::NixosLinux
        | data::NodeModel::KaliLinux
        | data::NodeModel::InfrahubServer
        | data::NodeModel::JenkinsServer
        | data::NodeModel::NautobotServer
        | data::NodeModel::NetboxServer
        | data::NodeModel::SignozServer
        | data::NodeModel::VirtServer
        | data::NodeModel::Generic
        | data::NodeModel::FreeBsd
        | data::NodeModel::OpenBsd => linux_cloud_init(node, node_image, ztp, &dir),
        data::NodeModel::DevboxWindows | data::NodeModel::WindowsServer => {
            windows_cloudbase_init(node, ztp, &dir)
        }
        data::NodeModel::AlpineLinux => alpine_cloud_init(node, ztp, &dir),
        _ => {
            bail!(
                "Cloud-Init ZTP method not supported for {}",
                node_image.model
            );
        }
    }
}

/// The files cloud-init writes and the commands it runs on a Linux or BSD node: certificates,
/// text files and the guest login's environment file, staged and moved into place first, then
/// startup scripts.
fn cloud_init_files(
    node: &topology::NodeExpanded,
    cert_paths: Option<&NodeCertPaths>,
    guest_username: &str,
) -> Result<(Vec<template::CloudInitWriteFile>, Vec<String>)> {
    let mut write_files = match cert_paths {
        Some(certs) => build_cloud_init_cert_files(certs, NODE_CERTS_DIR)?,
        None => vec![],
    };

    // Stage text_files and the owner's environment file, and move them into place first in
    // runcmd, once their owners exist.
    let mut staged_files = build_cloud_init_text_files(node.text_files.as_deref());
    let env_vars = match node.environment_variables.as_deref() {
        Some(env_vars) if !env_vars.is_empty() => Some(env_vars),
        _ => None,
    };
    if let Some(env_vars) = env_vars {
        staged_files.push(build_user_env_file(env_vars, guest_username));
    }
    let (staged_writes, mut runcmd) =
        stage_cloud_init_files(&staged_files, NODE_STAGING_DIR, NODE_PASSWD_FILE);
    write_files.extend(staged_writes);

    // Load the environment file from the owner's login shell.
    if env_vars.is_some() {
        runcmd.push(user_env_loader_runcmd(guest_username, NODE_PASSWD_FILE));
    }

    let (script_files, script_runcmd) =
        build_cloud_init_scripts(&node.name, node.startup_scripts.as_deref())?;
    write_files.extend(script_files);
    runcmd.extend(script_runcmd);
    Ok((write_files, runcmd))
}

/// Write a node's cloud-init seed into `dir`, its user data private, and build its ISO.
fn write_cloud_init_seed(
    dir: &str,
    user_data: &str,
    meta_data: &str,
    network_config: &str,
) -> Result<()> {
    util::create_dir_with_permissions(Path::new(dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&format!("{dir}/{CLOUD_INIT_USER_DATA}")),
        user_data.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;
    util::write_file_with_permissions(
        Path::new(&format!("{dir}/{CLOUD_INIT_META_DATA}")),
        meta_data.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;
    util::write_file_with_permissions(
        Path::new(&format!("{dir}/{CLOUD_INIT_NETWORK_CONFIG}")),
        network_config.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;
    create_iso(dir, format!("{dir}/{ZTP_ISO}"), ZTP_ISO_VOLUME_ID)?;
    Ok(())
}

/// Cloud-init for the Linux and BSD VMs: the owner's login, with its admin rights, and the
/// node's files and scripts.
fn linux_cloud_init(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
    dir: &str,
) -> Result<()> {
    let NodeZtp {
        mgmt_net,
        node_ipv4_address,
        mac_address,
        cert_paths,
        guest_account,
        ..
    } = *ztp;
    let account = guest_account.with_context(|| {
        format!(
            "Node '{}' has no owner account for its cloud-init",
            node.name
        )
    })?;
    let (mut write_files, runcmd) = cloud_init_files(node, cert_paths, &account.username)?;
    if let Some(doas_file) = cloud_init_doas_file(node.model_kind) {
        write_files.push(doas_file);
    }

    let mut user = match (
        node.model_kind.model(),
        &vm_settings(node_image)?.os_variant,
    ) {
        (data::NodeModel::NixosLinux, _) => guest_account::nixos_cloud_init_user(account)?,
        (_, data::OsVariant::Bsd) => {
            guest_account::bsd_cloud_init_user(account, node.model_kind.model())?
        }
        _ => guest_account::cloud_init_user(account)?,
    };
    user.sudo = cloud_init_sudo_rule(node.model_kind);
    let users = vec![user];

    let cloud_init_config = template::CloudInitConfig {
        ca_certs: cloud_init_ca_certs(cert_paths)?,
        hostname: node.name.clone(),
        fqdn: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
        manage_etc_hosts: cloud_init_manage_etc_hosts(node.model_kind),
        ssh_pwauth: cloud_init_ssh_pwauth(node.model_kind),
        users,
        write_files: (!write_files.is_empty()).then_some(write_files),
        runcmd: (!runcmd.is_empty()).then_some(runcmd),
        ..Default::default()
    };
    let meta_data = template::MetaDataConfig {
        instance_id: format!("iid-{}", node.name),
        local_hostname: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
        ..Default::default()
    };
    let network_config = template::CloudInitNetwork::ztp_interface(
        node_ipv4_address,
        mac_address.to_string(),
        &mgmt_net.v4,
        cloud_init_ipv6_address(node.model_kind, node.ipv6_address),
        mgmt_net.v6.as_ref(),
    );
    write_cloud_init_seed(
        dir,
        &cloud_init_config.render()?,
        &meta_data.render()?,
        &network_config.render()?,
    )
}

/// The startup scripts of a Windows node, written under `C:\sherpa\startup_scripts` and run
/// by cloudbase-init.
fn windows_startup_scripts(
    node: &topology::NodeExpanded,
) -> Result<(Vec<template::CloudbaseWriteFile>, Vec<String>)> {
    let mut write_files = Vec::new();
    let mut runcmd = Vec::new();
    for script in node.startup_scripts.iter().flatten() {
        let contents = util::base64_decode(&script.content).with_context(|| {
            format!(
                "Failed to decode startup_script '{}' for node '{}'",
                script.filename, node.name
            )
        })?;

        let target_path = format!(r"C:\sherpa\startup_scripts\{}", script.filename);

        write_files.push(template::CloudbaseWriteFile {
            path: target_path.clone(),
            content: contents,
            permissions: "0755".to_string(),
        });

        runcmd.push(format!(
            r#"powershell -ExecutionPolicy Bypass -File "{target_path}""#
        ));
    }
    Ok((write_files, runcmd))
}

/// Cloudbase-init for the Windows VMs: the owner's login, a local administrator with its own
/// SSH keys file, and the node's files, startup scripts and machine environment.
///
/// The owner is the node's only enabled login. The image's built-in `Administrator` and the
/// `Admin` account cloudbase-init's `CreateUserPlugin` makes before the user data runs (its
/// `username` setting, which `NoCloud` metadata cannot change) are disabled by
/// [`WINDOWS_OWNER_ONLY_LOGIN_SCRIPT`], before the node's own startup scripts run.
fn windows_cloudbase_init(
    node: &topology::NodeExpanded,
    ztp: &NodeZtp<'_>,
    dir: &str,
) -> Result<()> {
    let NodeZtp {
        mgmt_net,
        node_ipv4_address,
        mac_address,
        cert_paths,
        guest_account,
        ..
    } = *ztp;
    let account = guest_account.with_context(|| {
        format!(
            "Node '{}' has no owner account for its cloudbase-init",
            node.name
        )
    })?;
    let users = vec![guest_account::cloudbase_user(account)];

    // The account reads its own keys file, private to Administrators and SYSTEM.
    let sshd_config = template::WindowsSshdConfigTemplate {
        authorized_keys_dir: WINDOWS_SSHD_AUTHORIZED_KEYS_DIR.to_owned(),
        ready_marker: SHERPA_SSHD_READY_MARKER.to_owned(),
    };
    let owner_only_login = template::WindowsOwnerOnlyLoginTemplate {
        owner: account.username.clone(),
    };
    let mut write_files = vec![
        template::CloudbaseWriteFile {
            path: WINDOWS_SSHD_CONFIG.to_owned(),
            content: sshd_config.render()?,
            permissions: "0644".to_string(),
        },
        template::CloudbaseWriteFile {
            path: WINDOWS_OWNER_ONLY_LOGIN_SCRIPT.to_owned(),
            content: owner_only_login.render()?,
            permissions: "0644".to_string(),
        },
        template::CloudbaseWriteFile {
            path: format!(r"{WINDOWS_AUTHORIZED_KEYS_DIR}\{}", account.username),
            content: format!("{}\n", account.ssh_keys.join("\n")),
            permissions: "0600".to_string(),
        },
    ];
    if let Some(certs) = cert_paths {
        write_files.extend(build_cloudbase_cert_files(certs, NODE_CERTS_DIR_WINDOWS)?);
    }
    write_files.extend(build_cloudbase_text_files(node.text_files.as_deref()));

    let mut runcmd = build_windows_private_file_acls(&write_files)
        .with_context(|| format!("Failed to write files for node '{}'", node.name))?;
    runcmd.push(r#"powershell -Command "Restart-Service sshd""#.to_string());
    runcmd.push(format!(
        r#"powershell -ExecutionPolicy Bypass -File "{WINDOWS_OWNER_ONLY_LOGIN_SCRIPT}""#
    ));
    let (script_files, script_runcmd) = windows_startup_scripts(node)?;
    write_files.extend(script_files);
    runcmd.extend(script_runcmd);

    // Inject environment_variables as machine-level env vars
    if let Some(ref env_vars) = node.environment_variables
        && !env_vars.is_empty()
    {
        runcmd.extend(build_windows_env_commands(env_vars));
    }

    let cloudbase_config = template::CloudbaseInitConfig {
        set_hostname: node.name.clone(),
        users,
        write_files,
        runcmd,
    };
    let meta_data = template::MetaDataConfig {
        instance_id: format!("iid-{}", node.name),
        local_hostname: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
        ..Default::default()
    };
    let network_config = template::CloudbaseInitNetwork::ztp_interface(
        node_ipv4_address,
        mac_address.to_string(),
        &mgmt_net.v4,
        node.ipv6_address,
        mgmt_net.v6.as_ref(),
    );
    write_cloud_init_seed(
        dir,
        &cloudbase_config.render()?,
        &meta_data.render()?,
        &network_config.render()?,
    )
}

/// Cloud-init for Alpine: `wheel` with doas, and the lab DNS written to `resolv.conf`.
fn alpine_cloud_init(node: &topology::NodeExpanded, ztp: &NodeZtp<'_>, dir: &str) -> Result<()> {
    let NodeZtp {
        mgmt_net,
        node_ipv4_address,
        mac_address,
        cert_paths,
        guest_account,
        ..
    } = *ztp;
    let account = guest_account.with_context(|| {
        format!(
            "Node '{}' has no owner account for its cloud-init",
            node.name
        )
    })?;
    let (mut write_files, runcmd) = cloud_init_files(node, cert_paths, &account.username)?;
    write_files.push(guest_account::doas_file(account));
    let users = vec![guest_account::alpine_cloud_init_user(account)?];

    let cloud_init_config = template::CloudInitConfig {
        ca_certs: cloud_init_ca_certs(cert_paths)?,
        hostname: node.name.clone(),
        fqdn: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
        manage_etc_hosts: cloud_init_manage_etc_hosts(node.model_kind),
        ssh_pwauth: cloud_init_ssh_pwauth(node.model_kind),
        // Alpine's network renderer leaves DNS to a resolvconf tool the image lacks.
        manage_resolv_conf: Some(true),
        resolv_conf: Some(template::CloudInitResolvConf::lab_dns(
            mgmt_net.v4.boot_server,
        )),
        packages: None,
        users,
        write_files: (!write_files.is_empty()).then_some(write_files),
        runcmd: (!runcmd.is_empty()).then_some(runcmd),
    };
    let meta_data = template::MetaDataConfig {
        instance_id: format!("iid-{}", node.name),
        local_hostname: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
        ..Default::default()
    };
    let network_config = template::CloudInitNetwork::ztp_interface(
        node_ipv4_address,
        mac_address.to_string(),
        &mgmt_net.v4,
        node.ipv6_address,
        mgmt_net.v6.as_ref(),
    );
    write_cloud_init_seed(
        dir,
        &cloud_init_config.render()?,
        &meta_data.render()?,
        &network_config.render()?,
    )
}

#[instrument(level = "debug", skip_all, fields(node_name = %node.name))]
fn generate_tftp_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
    custom_ztp: Option<&str>,
) -> Result<()> {
    let NodeZtp {
        tftp_dir,
        dns,
        mgmt_net,
        node_ipv4_address,
        progress,
        guest_account,
        ..
    } = *ztp;
    let _ = progress.send_status(
        format!("Creating TFTP ZTP config for VM: {}", node.name),
        StatusKind::Progress,
    );

    if let Some(custom_config) = custom_ztp {
        let ztp_config = format!("{tftp_dir}/{}.conf", node.name);
        util::write_file_with_permissions(
            Path::new(&ztp_config),
            custom_config.as_bytes(),
            TFTP_CONFIG_FILE_MODE,
        )?;
    } else {
        match node.model_kind.model() {
            data::NodeModel::AristaEos => {
                let arista_template = template::AristaEosVmZtpTemplate {
                    hostname: node.name.clone(),
                    owner: guest_account::owner_user(
                        guest_account,
                        KeyFormat::OpenSsh,
                        PasswordForm::Hashed(CryptHash::Sha512),
                    )?,
                    dns: dns.clone(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                let rendered_template = arista_template.render()?;
                let ztp_config = format!("{tftp_dir}/{}.conf", node.name);
                util::write_file_with_permissions(
                    Path::new(&ztp_config),
                    rendered_template.as_bytes(),
                    TFTP_CONFIG_FILE_MODE,
                )?;
            }
            data::NodeModel::ArubaAoscx => {
                // AOS-CX takes the owner's keys only: its config carries no device password.
                let data::OwnerUser {
                    username, ssh_keys, ..
                } = guest_account::owner_user(
                    guest_account,
                    KeyFormat::OpenSsh,
                    PasswordForm::Plain,
                )?;
                let aruba_template = template::ArubaAoscxTemplate {
                    hostname: node.name.clone(),
                    username,
                    ssh_keys,
                    dns: dns.clone(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                let rendered_template = aruba_template.render()?;
                let ztp_config = format!("{tftp_dir}/{}.conf", node.name);
                util::write_file_with_permissions(
                    Path::new(&ztp_config),
                    rendered_template.as_bytes(),
                    TFTP_CONFIG_FILE_MODE,
                )?;
            }
            data::NodeModel::JuniperEvolved => {
                let juniper_template = template::JunipervJunosZtpTemplate {
                    hostname: node.name.clone(),
                    owner: guest_account::owner_user(
                        guest_account,
                        KeyFormat::Junos,
                        PasswordForm::Hashed(CryptHash::Sha512),
                    )?,
                    mgmt_interface: node_image.interfaces.management.to_string(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                let juniper_rendered_template = juniper_template.render()?;
                let ztp_config = format!("{tftp_dir}/{}.conf", node.name);
                util::write_file_with_permissions(
                    Path::new(&ztp_config),
                    juniper_rendered_template.as_bytes(),
                    TFTP_CONFIG_FILE_MODE,
                )?;
            }
            _ => {
                bail!("TFTP ZTP method not supported for {}", node_image.model);
            }
        }
    }

    Ok(())
}

#[instrument(level = "debug", skip_all, fields(node_name = %node.name))]
fn generate_cdrom_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
    custom_ztp: Option<&str>,
) -> Result<()> {
    let _ = ztp.progress.send_status(
        format!("Creating CDROM ZTP config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let dir = ztp.boot_dir.to_owned();
    // PAN-OS boots from a bootstrap layout of its own, not one config file.
    if node.model_kind.model() == data::NodeModel::PaloaltoPanos {
        return write_panos_bootstrap(node, ztp, custom_ztp, &dir);
    }
    let config = match custom_ztp {
        Some(custom_config) => custom_config.to_owned(),
        None => cdrom_config(node, node_image, ztp)?,
    };
    let ztp_config = format!("{dir}/{}", ztp_config_filename(node.model_kind)?);
    util::create_dir_with_permissions(Path::new(&dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&ztp_config),
        config.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;
    create_iso(&dir, format!("{dir}/{ZTP_ISO}"), ZTP_ISO_VOLUME_ID)?;
    Ok(())
}

/// The config a CD-ROM ZTP node reads from its ISO, as `ztp_config_filename` names it.
fn cdrom_config(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
) -> Result<String> {
    let NodeZtp {
        dns,
        mgmt_net,
        node_ipv4_address,
        guest_account,
        ..
    } = *ztp;
    match node.model_kind.model() {
        data::NodeModel::CiscoCsr1000
        | data::NodeModel::CiscoCat8000
        | data::NodeModel::CiscoCat9000 => iosxe_config(node, node_image, ztp),
        data::NodeModel::CiscoAsa => asav_config(node, ztp),
        data::NodeModel::CiscoNexus9300 => Ok(template::CiscoNxosZtpTemplate {
            hostname: node.name.clone(),
            owner: guest_account::owner_user(
                guest_account,
                KeyFormat::OpenSsh,
                PasswordForm::Plain,
            )?,
            dns: dns.clone(),
            mgmt_ipv4_address: Some(node_ipv4_address),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?),
        data::NodeModel::CiscoIosxr9000 => Ok(template::CiscoIosxrZtpTemplate {
            hostname: node.name.clone(),
            owner: guest_account::owner_user(
                guest_account,
                KeyFormat::OpenSsh,
                PasswordForm::Plain,
            )?,
            dns: dns.clone(),
            mgmt_ipv4_address: Some(node_ipv4_address),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?),
        data::NodeModel::CiscoFtd => ftdv_config(node, ztp),
        data::NodeModel::JuniperSrxv3
        | data::NodeModel::JuniperRouter
        | data::NodeModel::JuniperSwitch => Ok(template::JunipervJunosZtpTemplate {
            hostname: node.name.clone(),
            owner: guest_account::owner_user(
                guest_account,
                KeyFormat::Junos,
                PasswordForm::Hashed(CryptHash::Sha512),
            )?,
            mgmt_interface: node_image.interfaces.management.to_string(),
            mgmt_ipv4_address: Some(node_ipv4_address),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?),
        _ => {
            bail!("CDROM ZTP method not supported for {}", node_image.model);
        }
    }
}

/// IOS-XE: the owner's keys as their MD5 hashes, and the Cat8000v and Cat9000v boot licences.
fn iosxe_config(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
) -> Result<String> {
    let NodeZtp {
        dns,
        mgmt_net,
        node_ipv4_address,
        guest_account,
        ..
    } = *ztp;
    let license_boot_command = match node.model_kind.model() {
        data::NodeModel::CiscoCat8000 => {
            Some("license boot level network-premier addon dna-premier".to_string())
        }
        data::NodeModel::CiscoCat9000 => {
            Some("license boot level network-advantage addon dna-advantage".to_string())
        }
        _ => None,
    };
    Ok(template::CiscoIosXeZtpTemplate {
        hostname: node.name.clone(),
        owner: guest_account::owner_user(guest_account, KeyFormat::CiscoMd5, PasswordForm::Plain)?,
        mgmt_interface: node_image.interfaces.management.to_string(),
        dns: dns.clone(),
        license_boot_command,
        mgmt_ipv4_address: Some(node_ipv4_address),
        mgmt_ipv4: mgmt_net.v4.clone(),
        mgmt_ipv6_address: node.ipv6_address,
        mgmt_ipv6: mgmt_net.v6.clone(),
    }
    .render()?)
}

/// `ASAv`: the owner's key as its SHA-256 hash, and the device password as the owner's and the
/// enable password.
fn asav_config(node: &topology::NodeExpanded, ztp: &NodeZtp<'_>) -> Result<String> {
    let NodeZtp {
        dns,
        mgmt_net,
        node_ipv4_address,
        guest_account,
        ..
    } = *ztp;
    Ok(template::CiscoAsaZtpTemplate {
        hostname: node.name.clone(),
        owner: guest_account::owner_user(
            guest_account,
            KeyFormat::CiscoSha256,
            PasswordForm::Plain,
        )?,
        dns: dns.clone(),
        mgmt_ipv4_address: Some(node_ipv4_address),
        mgmt_ipv4: mgmt_net.v4.clone(),
        mgmt_ipv6_address: node.ipv6_address,
        mgmt_ipv6: mgmt_net.v6.clone(),
    }
    .render()?)
}

/// `FTDv`: its day-0 JSON, managed locally. Its only account, `admin`, takes the lab owner's
/// device password.
fn ftdv_config(node: &topology::NodeExpanded, ztp: &NodeZtp<'_>) -> Result<String> {
    let NodeZtp {
        mgmt_net,
        node_ipv4_address,
        device_password,
        ..
    } = *ztp;
    Ok(serde_json::to_string(&template::CiscoFtdZtpTemplate {
        eula: "accept".to_string(),
        hostname: node.name.clone(),
        admin_password: guest_account::device_password(device_password, PasswordForm::Plain)?,
        dns1: Some(mgmt_net.v4.boot_server),
        ipv4_mode: Some(template::CiscoFxosIpMode::Manual),
        ipv4_addr: Some(node_ipv4_address),
        ipv4_gw: Some(mgmt_net.v4.first),
        ipv4_mask: Some(mgmt_net.v4.subnet_mask),
        manage_locally: template::YesNo(true),
        ..Default::default()
    })?)
}

/// PAN-OS bootstrap: the layout PA-VM reads, on an ISO with the bootstrap volume label. Its
/// `config` folder holds the node's own config as `init-cfg.txt`, or else the generated
/// `init-cfg.txt` and `bootstrap.xml`.
fn write_panos_bootstrap(
    node: &topology::NodeExpanded,
    ztp: &NodeZtp<'_>,
    custom_ztp: Option<&str>,
    dir: &str,
) -> Result<()> {
    // PA-VM requires config/, content/, license/, software/ directories
    let config_subdir = format!("{dir}/config");
    util::create_dir_with_permissions(Path::new(&config_subdir), NODE_CONFIG_DIR_MODE)?;
    util::create_dir_with_permissions(Path::new(&format!("{dir}/content")), NODE_CONFIG_DIR_MODE)?;
    util::create_dir_with_permissions(Path::new(&format!("{dir}/license")), NODE_CONFIG_DIR_MODE)?;
    util::create_dir_with_permissions(Path::new(&format!("{dir}/software")), NODE_CONFIG_DIR_MODE)?;

    match custom_ztp {
        Some(custom_config) => util::write_file_with_permissions(
            Path::new(&format!("{config_subdir}/{PALOALTO_ZTP_CONFIG}")),
            custom_config.as_bytes(),
            NODE_CONFIG_FILE_MODE,
        )?,
        None => write_panos_generated_config(node, ztp, &config_subdir)?,
    }

    create_iso(
        dir,
        format!("{dir}/{ZTP_ISO}"),
        PALOALTO_BOOTSTRAP_VOLUME_ID,
    )?;
    Ok(())
}

/// The config Sherpa generates for PAN-OS, `init-cfg.txt` and `bootstrap.xml`, written into the
/// bootstrap's `config` folder.
fn write_panos_generated_config(
    node: &topology::NodeExpanded,
    ztp: &NodeZtp<'_>,
    config_subdir: &str,
) -> Result<()> {
    let NodeZtp {
        mgmt_net,
        node_ipv4_address,
        guest_account,
        ..
    } = *ztp;
    // init-cfg.txt — network and hostname settings
    let init_cfg = template::PaloAltoPanosZtpTemplate {
        hostname: node.name.clone(),
        mgmt_ipv4_address: node_ipv4_address,
        mgmt_ipv6_address: node.ipv6_address,
        mgmt_ipv6: mgmt_net.v6.clone(),
        mgmt_netmask: mgmt_net.v4.subnet_mask,
        mgmt_gateway: mgmt_net.v4.first,
        dns_primary: mgmt_net.v4.boot_server,
        dns_secondary: mgmt_net.v4.boot_server,
    };
    util::write_file_with_permissions(
        Path::new(&format!("{config_subdir}/{PALOALTO_ZTP_CONFIG}")),
        init_cfg.render()?.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    // bootstrap.xml — the running config, with `admin` and the owner. PAN-OS stores a
    // SHA-256-crypt hash of the password, and the whole SSH key line base64-encoded.
    let bootstrap = template::PaloAltoPanosBootstrapTemplate {
        hostname: node.name.clone(),
        owner: guest_account::owner_user(
            guest_account,
            KeyFormat::PanosBase64,
            PasswordForm::Hashed(CryptHash::Sha256),
        )?,
        mgmt_ipv4_address: node_ipv4_address,
        mgmt_ipv6_address: node.ipv6_address,
        mgmt_ipv6: mgmt_net.v6.clone(),
        mgmt_netmask: mgmt_net.v4.subnet_mask,
        mgmt_gateway: mgmt_net.v4.first,
        dns_primary: mgmt_net.v4.boot_server,
    };
    util::write_file_with_permissions(
        Path::new(&format!("{config_subdir}/{PALOALTO_BOOTSTRAP_CONFIG}")),
        bootstrap.render()?.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;
    Ok(())
}

#[instrument(level = "debug", skip_all, fields(node_name = %node.name))]
fn generate_disk_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
    custom_ztp: Option<&str>,
) -> Result<(String, String)> {
    let NodeZtp {
        images_dir,
        node_name_with_lab,
        progress,
        ..
    } = *ztp;
    let _ = progress.send_status(
        format!("Creating Disk ZTP config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let dir = ztp.boot_dir.to_owned();
    match node.model_kind.model() {
        data::NodeModel::CiscoIos | data::NodeModel::CiscoIosl2 => {
            let rendered_template = match custom_ztp {
                Some(config) => config.to_owned(),
                None => ios_config(node, node_image, ztp)?,
            };
            let ztp_config = format!("{dir}/{CISCO_IOS_ZTP_CONFIG}");
            util::create_dir_with_permissions(Path::new(&dir), NODE_CONFIG_DIR_MODE)?;
            util::write_file_with_permissions(
                Path::new(&ztp_config),
                rendered_template.as_bytes(),
                NODE_CONFIG_FILE_MODE,
            )?;

            let src_disk = format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_IOSV}");
            let dst_disk = format!("{dir}/{}-cfg.img", node.name);

            util::copy_file_with_permissions(&src_disk, &dst_disk, NODE_CONFIG_FILE_MODE)?;
            util::copy_to_dos_image(&ztp_config, &dst_disk, "/")?;

            let dst_final = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img");
            Ok((dst_disk, dst_final))
        }
        data::NodeModel::CiscoIse => ise_config_disk(node, ztp, custom_ztp, &dir),
        data::NodeModel::MikrotikChr => {
            mikrotik_config_disk(node, node_image, ztp, custom_ztp, &dir)
        }
        _ => {
            bail!("Disk ZTP method not supported for {}", node_image.model);
        }
    }
}

/// `IOSv` and `IOSvL2`: the owner's keys as their MD5 hashes.
fn ios_config(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
) -> Result<String> {
    let NodeZtp {
        dns,
        mgmt_net,
        node_ipv4_address,
        guest_account,
        ..
    } = *ztp;
    let owner = guest_account::owner_user(guest_account, KeyFormat::CiscoMd5, PasswordForm::Plain)?;
    let mgmt_interface = node_image.interfaces.management.to_string();
    let rendered = if node.model_kind.model() == data::NodeModel::CiscoIosl2 {
        template::CiscoIosl2ZtpTemplate {
            hostname: node.name.clone(),
            owner,
            mgmt_interface,
            dns: dns.clone(),
            mgmt_ipv4_address: Some(node_ipv4_address),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?
    } else {
        template::CiscoIosZtpTemplate {
            hostname: node.name.clone(),
            owner,
            mgmt_interface,
            dns: dns.clone(),
            mgmt_ipv4_address: Some(node_ipv4_address),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?
    };
    Ok(rendered)
}

/// ISE: its config on an ext4 disk. ISE has a single administrator, the owner, with the device
/// password and their first RSA key.
fn ise_config_disk(
    node: &topology::NodeExpanded,
    ztp: &NodeZtp<'_>,
    custom_ztp: Option<&str>,
    dir: &str,
) -> Result<(String, String)> {
    let NodeZtp {
        images_dir,
        dns,
        mgmt_net,
        node_ipv4_address,
        node_name_with_lab,
        guest_account,
        ..
    } = *ztp;
    let rendered_template = if let Some(config) = custom_ztp {
        config.to_owned()
    } else {
        template::CiscoIseZtpTemplate {
            hostname: node.name.clone(),
            owner: guest_account::owner_user(
                guest_account,
                KeyFormat::OpenSsh,
                PasswordForm::Plain,
            )?,
            dns: dns.clone(),
            mgmt_ipv4_address: node_ipv4_address,
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?
    };
    let ztp_config = format!("{dir}/{CISCO_ISE_ZTP_CONFIG}");
    util::create_dir_with_permissions(Path::new(dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&ztp_config),
        rendered_template.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    let src_disk = format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_ISE}");
    let dst_disk = format!("{dir}/{node_name_with_lab}-cfg.img");

    util::copy_file_with_permissions(&src_disk, &dst_disk, NODE_CONFIG_FILE_MODE)?;
    util::copy_to_ext4_image(&[&ztp_config], &dst_disk, "/")?;

    let dst_final = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img");
    Ok((dst_disk, dst_final))
}

/// `RouterOS`: its config and the SSH keys on a FAT32 disk. `RouterOS` imports one key per file,
/// so each owner key gets its own file on the disk.
fn mikrotik_config_disk(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
    custom_ztp: Option<&str>,
    dir: &str,
) -> Result<(String, String)> {
    let NodeZtp {
        images_dir,
        dns,
        mgmt_net,
        node_ipv4_address,
        node_name_with_lab,
        guest_account,
        ..
    } = *ztp;
    let mut owner_key_files = Vec::new();
    let rendered_template = if let Some(config) = custom_ztp {
        config.to_owned()
    } else {
        let owner =
            guest_account::owner_user(guest_account, KeyFormat::OpenSsh, PasswordForm::Plain)?;
        let mut owner_key_paths = Vec::new();
        for (index, key) in owner.ssh_keys.iter().enumerate() {
            let filename = template::MikrotikRouterosZtpTemplate::owner_key_filename(
                &owner.username,
                index + 1,
            );
            owner_key_files.push((
                format!("{dir}/{filename}"),
                format!("{} {}\n", key.kind, key.value),
            ));
            owner_key_paths.push(template::MikrotikRouterosZtpTemplate::owner_key_path(
                &owner.username,
                index + 1,
            ));
        }
        template::MikrotikRouterosZtpTemplate {
            hostname: node.name.clone(),
            owner,
            owner_key_files: owner_key_paths,
            mgmt_interface: node_image.interfaces.management.to_string(),
            dns: dns.clone(),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv4_address: Some(node_ipv4_address),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?
    };
    let ztp_config = format!("{dir}/{MIKROTIK_CHR_ZTP_CONFIG}");
    util::create_dir_with_permissions(Path::new(dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&ztp_config),
        rendered_template.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    let src_disk = format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_FAT32}");
    let dst_disk = format!("{dir}/{}-cfg.img", node.name);

    util::copy_file_with_permissions(&src_disk, &dst_disk, NODE_CONFIG_FILE_MODE)?;
    util::copy_to_dos_image(&ztp_config, &dst_disk, "/")?;
    for (path, content) in owner_key_files {
        util::write_file_with_permissions(
            Path::new(&path),
            content.as_bytes(),
            NODE_CONFIG_FILE_MODE,
        )?;
        util::copy_to_dos_image(&path, &dst_disk, "/")?;
    }

    let dst_final = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img");
    Ok((dst_disk, dst_final))
}

#[instrument(level = "debug", skip_all, fields(node_name = %node.name))]
fn generate_usb_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
    custom_ztp: Option<&str>,
) -> Result<(String, String)> {
    let NodeZtp {
        images_dir,
        dns,
        mgmt_net,
        node_ipv4_address,
        node_name_with_lab,
        progress,
        guest_account,
        ..
    } = *ztp;
    let _ = progress.send_status(
        format!("Creating USB ZTP config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let dir = ztp.boot_dir.to_owned();

    match node_image.model {
        data::NodeModel::CumulusLinux => {
            let rendered_template = if let Some(config) = custom_ztp {
                config.to_owned()
            } else {
                let t = template::CumulusLinuxZtpTemplate {
                    hostname: node.name.clone(),
                    owner: guest_account::owner_user(
                        guest_account,
                        KeyFormat::OpenSsh,
                        PasswordForm::Plain,
                    )?,
                    dns: dns.clone(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                t.render()?
            };
            let ztp_config = format!("{dir}/{CUMULUS_ZTP}");
            util::create_dir_with_permissions(Path::new(&dir), NODE_CONFIG_DIR_MODE)?;
            util::write_file_with_permissions(
                Path::new(&ztp_config),
                rendered_template.as_bytes(),
                NODE_CONFIG_FILE_MODE,
            )?;

            let src_usb = format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_FAT32}");
            let dst_usb = format!("{dir}/cfg.img");

            util::copy_file_with_permissions(&src_usb, &dst_usb, NODE_CONFIG_FILE_MODE)?;
            util::copy_to_dos_image(&ztp_config, &dst_usb, "/")?;

            let dst_final = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img");
            Ok((dst_usb, dst_final))
        }
        data::NodeModel::JuniperEvolved => {
            let rendered_template = if let Some(config) = custom_ztp {
                config.to_owned()
            } else {
                let t = template::JunipervJunosZtpTemplate {
                    hostname: node.name.clone(),
                    owner: guest_account::owner_user(
                        guest_account,
                        KeyFormat::Junos,
                        PasswordForm::Hashed(CryptHash::Sha512),
                    )?,
                    mgmt_interface: node_image.interfaces.management.to_string(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                t.render()?
            };
            let ztp_config = format!("{dir}/{JUNIPER_ZTP_CONFIG}");
            let ztp_config_tgz = format!("{dir}/{JUNIPER_ZTP_CONFIG_TGZ}");

            util::create_dir_with_permissions(Path::new(&dir), NODE_CONFIG_DIR_MODE)?;
            util::write_file_with_permissions(
                Path::new(&ztp_config),
                rendered_template.as_bytes(),
                NODE_CONFIG_FILE_MODE,
            )?;

            let src_usb = format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_JUNOS}");
            let dst_usb = format!("{dir}/cfg.img");

            util::copy_file_with_permissions(&src_usb, &dst_usb, NODE_CONFIG_FILE_MODE)?;
            util::create_config_archive(&ztp_config, &ztp_config_tgz, NODE_CONFIG_FILE_MODE)?;
            util::copy_to_dos_image(&ztp_config_tgz, &dst_usb, "/")?;
            util::copy_to_dos_image(&ztp_config, &dst_usb, "/")?;

            let dst_final = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img");
            Ok((dst_usb, dst_final))
        }
        _ => {
            bail!("USB ZTP method not supported for {}", node_image.model);
        }
    }
}

#[instrument(level = "debug", skip_all, fields(node_name = %node.name))]
fn generate_http_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
) -> Result<()> {
    let NodeZtp {
        lab_dir,
        mgmt_net,
        node_ipv4_address,
        progress,
        guest_account,
        ..
    } = *ztp;
    let _ = progress.send_status(
        format!("Creating HTTP ZTP config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let dir = format!("{lab_dir}/{ZTP_DIR}/{NODE_CONFIGS_DIR}");

    match node_image.model {
        data::NodeModel::SonicLinux => {
            let sonic_ztp_file_map =
                template::SonicLinuxZtp::file_map(&node.name, &mgmt_net.v4.boot_server);

            let [ztp_init, ztp_config, ztp_user_script] =
                http_ztp_files(&node.name).map(|file| format!("{dir}/{file}"));
            let sonic_ztp = template::SonicLinuxZtp {
                hostname: node.name.clone(),
                mgmt_ipv4: mgmt_net.v4.clone(),
                mgmt_ipv4_address: Some(node_ipv4_address),
                mgmt_ipv6_address: node.ipv6_address,
                mgmt_ipv6: mgmt_net.v6.clone(),
            };
            util::create_dir_with_permissions(Path::new(&dir), NODE_CONFIG_DIR_MODE)?;
            util::write_file_with_permissions(
                Path::new(&ztp_init),
                sonic_ztp_file_map.as_bytes(),
                NODE_CONFIG_FILE_MODE,
            )?;
            util::write_file_with_permissions(
                Path::new(&ztp_config),
                sonic_ztp.config().as_bytes(),
                NODE_CONFIG_FILE_MODE,
            )?;

            let sonic_user_template = template::SonicLinuxUserTemplate {
                owner: guest_account::owner_user(
                    guest_account,
                    KeyFormat::OpenSsh,
                    PasswordForm::Hashed(CryptHash::Sha512),
                )?,
            };
            util::write_file_with_permissions(
                Path::new(&ztp_user_script),
                sonic_user_template.render()?.as_bytes(),
                NODE_CONFIG_FILE_MODE,
            )?;
        }
        _ => {
            bail!("HTTP ZTP method not supported for {}", node_image.model);
        }
    }

    Ok(())
}

/// The files a node that boots by HTTP is served from the lab's configs folder: its ZTP file
/// map, its config database and its account script.
fn http_ztp_files(node_name: &str) -> [String; 3] {
    [
        format!("{node_name}.conf"),
        format!("{node_name}_config_db.json"),
        template::SonicLinuxZtp::user_script_filename(node_name),
    ]
}

/// Remove the network boot files of node `node_name` in the lab folder `lab_dir` that its ZTP
/// method `method` does not use: its TFTP config unless it boots by TFTP, and its HTTP files
/// unless it boots by HTTP. They are left from a method it had before; the boot server would
/// still serve them. Other nodes' files are not touched.
///
/// # Errors
///
/// Fails when a file is there but cannot be removed.
#[instrument(level = "debug")]
pub(crate) fn remove_stale_network_boot_files(
    lab_dir: &str,
    node_name: &str,
    method: data::Ztp,
) -> Result<()> {
    let mut stale = Vec::new();
    if method != data::Ztp::Tftp {
        stale.push(format!("{lab_dir}/{ZTP_DIR}/{TFTP_DIR}/{node_name}.conf"));
    }
    if method != data::Ztp::Http {
        stale.extend(
            http_ztp_files(node_name)
                .map(|file| format!("{lab_dir}/{ZTP_DIR}/{NODE_CONFIGS_DIR}/{file}")),
        );
    }
    for path in stale {
        match fs::remove_file(&path) {
            Ok(()) => {
                tracing::info!(%path, "Removed a network boot file of the node's old ZTP method");
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to remove {path}"));
            }
        }
    }
    Ok(())
}

/// Ignition units for the systemd unit files the client read.
fn ignition_units(units: &[SystemdUnitData]) -> Result<Vec<template::IgnitionUnit>> {
    units
        .iter()
        .map(|unit| {
            let contents = util::base64_decode(&unit.content)
                .with_context(|| format!("systemd unit '{}' contents", unit.name))?;
            Ok(template::IgnitionUnit {
                name: unit.name.clone(),
                enabled: Some(unit.enabled),
                contents: Some(contents),
                ..Default::default()
            })
        })
        .collect()
}

#[instrument(level = "debug", skip_all, fields(node_name = %node.name))]
fn generate_ignition_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
) -> Result<(String, String, String, String)> {
    let _ = ztp.progress.send_status(
        format!("Creating Ignition config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let account = ztp.guest_account.with_context(|| {
        format!(
            "Node '{}' has no owner account for its Ignition config",
            node.name
        )
    })?;
    let ignition_user = guest_account::ignition_user(account)?;
    let mut files = ignition_base_files(&node.name);
    files.push(guest_account::sudoers_file(account));
    files.extend(ignition_text_files(node)?);
    let manifest_systemd_units = ignition_units(node.systemd_units.as_deref().unwrap_or_default())?;

    match node_image.model {
        data::NodeModel::FlatcarLinux => {
            flatcar_ignition(node, ztp, ignition_user, files, manifest_systemd_units)
        }
        _ => {
            bail!("Ignition ZTP method not supported for {}", node_image.model);
        }
    }
}

/// The files every Ignition node gets: its hostname, and updates off.
fn ignition_base_files(node_name: &str) -> Vec<template::IgnitionFile> {
    let hostname_file = template::IgnitionFile {
        path: "/etc/hostname".to_owned(),
        mode: template::FileMode::new(0o644),
        contents: Some(template::IgnitionFileContents::new(&format!(
            "data:,{node_name}"
        ))),
        ..Default::default()
    };
    vec![hostname_file, template::IgnitionFile::disable_updates()]
}

/// The manifest's text files as Ignition files, with their modes and owners.
fn ignition_text_files(node: &topology::NodeExpanded) -> Result<Vec<template::IgnitionFile>> {
    node.text_files
        .iter()
        .flatten()
        .map(|file| -> Result<template::IgnitionFile> {
            Ok(template::IgnitionFile {
                path: file.dst.clone(),
                mode: template::FileMode::new(
                    mode_from_octal_digits(file.permissions)
                        .with_context(|| format!("Text file {} mode is invalid", file.dst))?,
                ),
                overwrite: None,
                contents: Some(template::IgnitionFileContents::new(&format!(
                    "data:;base64,{}",
                    file.content
                ))),
                user: Some(template::IgnitionFileParams {
                    name: file.user.clone(),
                }),
                group: Some(template::IgnitionFileParams {
                    name: file.group.clone(),
                }),
                append: vec![],
            })
        })
        .collect()
}

/// Flatcar: its Ignition config, and a blank container data disk. Returns the config and the
/// data disk, each in the lab folder and in the storage pool.
fn flatcar_ignition(
    node: &topology::NodeExpanded,
    ztp: &NodeZtp<'_>,
    ignition_user: template::IgnitionUser,
    mut files: Vec<template::IgnitionFile>,
    manifest_systemd_units: Vec<template::IgnitionUnit>,
) -> Result<(String, String, String, String)> {
    let NodeZtp {
        images_dir,
        mgmt_net,
        node_ipv4_address,
        node_name_with_lab,
        cert_paths,
        ..
    } = *ztp;
    let dir = ztp.boot_dir.to_owned();
    let mut units = vec![template::IgnitionUnit::mount_container_disk()];
    units.extend(manifest_systemd_units);

    files.push(template::IgnitionFile::ztp_interface(
        node_ipv4_address,
        &mgmt_net.v4,
        node.ipv6_address,
        mgmt_net.v6.as_ref(),
    ));

    // Add TLS certificate files if cert paths are provided
    if let Some(certs) = cert_paths {
        files.extend(build_ignition_cert_files(certs, NODE_CERTS_DIR)?);
    }

    // Inject environment_variables into the owner's environment
    if let Some(ref env_vars) = node.environment_variables
        && !env_vars.is_empty()
    {
        files.extend(build_ignition_user_env(env_vars, &ignition_user.name));
    }

    let mut ignition_config = template::IgnitionConfig::new(
        vec![ignition_user],
        files,
        vec![],
        units,
        vec![],
        vec![template::IgnitionFileSystem::default()],
    );
    ignition_config
        .storage
        .directories
        .extend(build_ignition_home_directories(node.text_files.as_deref()));
    let flatcar_config = ignition_config.to_json_pretty()?;
    let src_ztp_file = format!("{dir}/{ZTP_JSON}");
    let dst_ztp_file = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.ign");

    util::create_dir_with_permissions(Path::new(&dir), NODE_CONFIG_DIR_MODE)?;
    util::write_file_with_permissions(
        Path::new(&src_ztp_file),
        flatcar_config.as_bytes(),
        NODE_CONFIG_FILE_MODE,
    )?;

    // Copy blank disk for container data
    let src_data_disk =
        format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_EXT4_500MB}");
    let dst_disk = format!("{dir}/{node_name_with_lab}-{CONTAINER_DISK_NAME}");

    util::copy_file_with_permissions(&src_data_disk, &dst_disk, NODE_CONFIG_FILE_MODE)?;

    let dst_config_disk =
        format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-{CONTAINER_DISK_NAME}");

    Ok((src_ztp_file, dst_ztp_file, dst_disk, dst_config_disk))
}

// ============================================================================
// Domain Template Building
// ============================================================================

/// Build a `DomainTemplate` from node data, image config, disks, interfaces, and networks.
#[instrument(
    level = "debug",
    skip(node, node_image, parts, firmware),
    fields(node_name = %node.name)
)]
pub fn build_domain_template(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_id: &str,
    qemu_bin: &str,
    parts: DomainParts,
    firmware: VmFirmware,
    resources: &data::NodeResources,
) -> Result<template::DomainTemplate> {
    let node_name_with_lab = format!("{}-{}", node.name, lab_id);
    let (video_model, video_vram) = match node_image.model {
        data::NodeModel::OmarchyLinux => ("virtio", None),
        _ => ("cirrus", Some(16384)),
    };
    // The resources stored on the node, so the domain and the database agree.
    let memory = resources
        .memory
        .with_context(|| format!("Node '{}' has no memory recorded", node.name))?;
    let cpu_count = resources
        .cpu_count
        .with_context(|| format!("Node '{}' has no vCPUs recorded", node.name))?;
    let vm = vm_settings(node_image)?;
    let cpu_model = match node.cpu.mode {
        Some(mode) => data::CpuModels::from(mode),
        None => vm.cpu.model.clone(),
    };
    let management_interface_type =
        management_interface_type_for_model(node.model_kind, &vm.interfaces.kind);

    Ok(template::DomainTemplate {
        qemu_bin: qemu_bin.to_string(),
        name: node_name_with_lab,
        memory,
        cpu_architecture: vm.cpu.architecture.clone(),
        cpu_model,
        machine_type: vm.machine.kind.clone(),
        cpu_count,
        vmx_enabled: vm.cpu.vmx,
        bios: vm.machine.bios.clone(),
        disks: parts.disks,
        interfaces: parts.interfaces,
        interface_type: vm.interfaces.kind.clone(),
        tuning: template::DomainTuning::from_node_settings(
            &node.cpu,
            &node.numa,
            &node.boot_disk,
            &node.interfaces,
            &management_interface_type,
        ),
        management_interface_type,
        reserved_interface_type: reserved_interface_type_for_model(
            node.model_kind,
            &vm.interfaces.kind,
        ),
        serial_socket: parts.consoles.serial,
        vnc_socket: parts.consoles.vnc,
        qemu_commands: firmware.qemu_commands,
        ignition_config: firmware.ignition_config,
        lab_id: lab_id.to_string(),
        management_network: parts.management_network,
        isolated_network: parts.isolated_network,
        reserved_network: parts.reserved_network,
        is_windows: matches!(
            node_image.model,
            data::NodeModel::WindowsServer | data::NodeModel::DevboxWindows
        ),
        cpu_features: cpu_features_for_model(node.model_kind),
        video_model: video_model.to_owned(),
        video_vram,
        watchdog: node.vm.as_ref().and_then(|vm| vm.watchdog),
    })
}

/// Return the management interface type for a node model.
/// Some platforms (e.g. IOS-XRv9000) require e1000 for the management NIC
/// while using virtio for data interfaces.
fn management_interface_type_for_model(
    model_kind: data::ModelKind,
    default: &data::InterfaceType,
) -> data::InterfaceType {
    match model_kind.model() {
        data::NodeModel::CiscoIosxr9000 => data::InterfaceType::E1000,
        _ => default.clone(),
    }
}

/// Return the reserved interface type for a node model.
/// Some platforms (e.g. IOS-XRv9000) use e1000 for the reserved system NICs
/// per Cisco documentation.
fn reserved_interface_type_for_model(
    model_kind: data::ModelKind,
    default: &data::InterfaceType,
) -> data::InterfaceType {
    match model_kind.model() {
        data::NodeModel::CiscoIosxr9000 => data::InterfaceType::E1000,
        _ => default.clone(),
    }
}

/// Return CPU feature overrides for specific node models.
fn cpu_features_for_model(model_kind: data::ModelKind) -> Vec<data::CpuFeature> {
    match model_kind.model() {
        data::NodeModel::JuniperSrxv3 => {
            // FreeBSD-based vSRX requires disabling several CPU features that
            // cause boot hangs on KVM. Matches vrnetlab's working configuration.
            let disabled = [
                "xsaveopt", "bmi1", "avx2", "bmi2", "erms", "invpcid", "rdseed", "adx", "smap",
                "abm",
            ];
            disabled
                .iter()
                .map(|name| data::CpuFeature {
                    name: (*name).to_owned(),
                    policy: data::CpuFeaturePolicy::Disable,
                })
                .collect()
        }
        _ => vec![],
    }
}

// ============================================================================
// Disk Cloning
// ============================================================================

/// The folder in the lab folder `lab_dir` that VM `node_name`'s boot files are written to, until
/// its media are in the storage pool: `vm-boot.d/<node>`. No node name has a `.`, so it is never
/// one of the lab's own folders or a node's.
pub(crate) fn vm_boot_dir(lab_dir: &str, node_name: &str) -> String {
    format!("{lab_dir}/{VM_BOOT_DIR}/{node_name}")
}

/// Node `node_name`'s own folder in the lab folder `lab_dir`, `<node>`: a container's config
/// mounts, and where releases before this one wrote a VM's boot files. `None` when the node is
/// named like one of the lab's own folders: that folder holds the lab's files too, so it is
/// never removed as the node's.
pub(crate) fn node_dir(lab_dir: &Path, node_name: &str) -> Option<PathBuf> {
    (!LAB_OWN_DIRS.contains(&node_name)).then(|| lab_dir.join(node_name))
}

/// Remove a VM's boot files, the folder `dir` that `vm_boot_dir` gives, or that `node_dir` gave
/// in releases before this one. Once its media are in
/// the storage pool nothing reads them again: QEMU reads the pool's copies. Returns whether
/// there was a folder to remove.
///
/// # Errors
///
/// Fails when the folder is there but cannot be removed.
#[instrument(fields(dir = %dir.display()), level = "debug")]
pub(crate) fn remove_vm_boot_dir(dir: &Path) -> Result<bool> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("Failed to remove {}", dir.display())),
    }
}

/// Clone a list of disks in parallel using libvirt. Once every disk is in the storage pool, the
/// VM boot file folders `boot_dirs` the media were made from are removed. When a disk fails
/// they are kept, so the failure can be looked at; `destroy` removes them with the lab folder.
#[instrument(skip(qemu_conn, clone_disks, progress, image_leases, boot_dirs), fields(%lab_id), level = "debug")]
pub async fn clone_node_disks(
    qemu_conn: Arc<libvirt::QemuConnection>,
    clone_disks: Vec<data::CloneDisk>,
    lab_id: &str,
    progress: &ProgressSender,
    image_leases: Vec<ImageLease>,
    boot_dirs: &[PathBuf],
) -> Result<()> {
    if clone_disks.is_empty() {
        let _ = progress.send_status("No disks to clone".to_string(), StatusKind::Info);
        return Ok(());
    }
    check_clone_destinations(&clone_disks)?;

    let disk_count = clone_disks.len();
    let _ = progress.send_status(
        format!("Cloning {disk_count} disks in parallel"),
        StatusKind::Progress,
    );

    let tasks: Vec<_> = clone_disks
        .into_iter()
        .map(|disk| {
            tokio::task::spawn(provision_disk(
                Arc::clone(&qemu_conn),
                disk,
                progress.clone(),
                image_leases.clone(),
                lab_id.to_string(),
            ))
        })
        .collect();

    // Wait for every task before rollback; no detached task may create a disk
    // after cleanup has completed.
    let mut created = Vec::new();
    let mut failures = Vec::new();
    for task in tasks {
        match task.await.context("Disk provisioning task failed") {
            Ok(Ok(path)) => created.push(path),
            Ok(Err(error)) | Err(error) => failures.push(format!("{error:#}")),
        }
    }
    if !failures.is_empty() {
        failures.extend(remove_created_disks(qemu_conn.clone(), created).await?);
        bail!("Disk provisioning failed: {}", failures.join("; "));
    }
    for dir in boot_dirs {
        // The disks are in the pool, so a folder left behind only costs disk space.
        if let Err(error) = remove_vm_boot_dir(dir) {
            tracing::warn!(%lab_id, error = format!("{error:#}"), "Failed to remove a VM's boot files");
        }
    }

    let _ = progress.send_status(
        "All disks cloned successfully".to_string(),
        StatusKind::Done,
    );

    Ok(())
}

/// Every disk goes to a destination of its own, which does not exist yet.
fn check_clone_destinations(clone_disks: &[data::CloneDisk]) -> Result<()> {
    let mut destinations = HashSet::new();
    for disk in clone_disks {
        anyhow::ensure!(
            destinations.insert(&disk.dst),
            "Duplicate disk destination: {}",
            disk.dst
        );
        anyhow::ensure!(
            fs::symlink_metadata(&disk.dst)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
            "Disk destination exists or cannot be inspected: {}",
            disk.dst
        );
    }
    Ok(())
}

/// Create one disk, holding the image leases until it exists. Returns its destination.
async fn provision_disk(
    conn: Arc<libvirt::QemuConnection>,
    disk: data::CloneDisk,
    progress: ProgressSender,
    leases: Vec<ImageLease>,
    lab_id: String,
) -> Result<String> {
    let dst = disk.dst.clone();
    let node_name = dst
        .split('/')
        .next_back()
        .and_then(|f| f.strip_suffix("-hdd.qcow2"))
        .unwrap_or("unknown");

    tracing::info!(
        lab_id = %lab_id,
        node_name = %node_name,
        src = %disk.src,
        "Cloning disk"
    );

    let _ = progress.send_status(
        format!("Cloning disk from: {}", disk.src),
        StatusKind::Progress,
    );

    tokio::task::spawn_blocking(move || -> Result<()> {
        let _leases = leases;
        create_disk(&conn, &disk)
    })
    .await
    .map_err(|e| anyhow!("Task join error: {e:?}"))??;

    let _ = progress.send_status(format!("Provisioned disk to: {dst}"), StatusKind::Done);
    Ok(dst)
}

/// Create a disk in libvirt: an overlay on its image, or a copy resized to its size. A copy
/// that cannot be resized is removed again.
fn create_disk(conn: &libvirt::QemuConnection, disk: &data::CloneDisk) -> Result<()> {
    let data::CloneDisk {
        src,
        dst,
        disk_size,
        method,
    } = disk;
    match method {
        data::DiskImageCloneMethod::Overlay => libvirt::create_overlay(conn, src, dst, *disk_size)?,
        data::DiskImageCloneMethod::Copy => {
            libvirt::clone_disk(conn, src, dst)
                .with_context(|| format!("Failed to copy {src} to {dst}"))?;
            if let Some(size_gb) = *disk_size
                && let Err(error) = libvirt::resize_disk(conn, dst, size_gb)
            {
                let name = Path::new(dst)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .context("Invalid disk filename")?;
                libvirt::delete_disk(conn, name).with_context(|| {
                    format!("Resize failed ({error:#}); cannot remove newly created disk {dst}")
                })?;
                return Err(error).context("Failed to resize copied disk");
            }
        }
    }
    Ok(())
}

/// Remove the disks a failed clone created, and return what could not be removed.
async fn remove_created_disks(
    conn: Arc<libvirt::QemuConnection>,
    created: Vec<String>,
) -> Result<Vec<String>> {
    tokio::task::spawn_blocking(move || {
        let mut errors = Vec::new();
        for path in created {
            if let Some(name) = Path::new(&path).file_name().and_then(|name| name.to_str())
                && let Err(error) = libvirt::delete_disk(&conn, name)
            {
                errors.push(format!(
                    "Cannot remove newly created disk {path}: {error:#}"
                ));
            }
        }
        errors
    })
    .await
    .context("Disk rollback task failed")
}

// ============================================================================
// VM Creation
// ============================================================================

/// Create a VM from a domain template using libvirt.
#[instrument(skip(qemu_conn, domain, progress), fields(vm_name = %domain.name), level = "debug")]
pub async fn create_vm(
    qemu_conn: Arc<libvirt::QemuConnection>,
    domain: template::DomainTemplate,
    progress: &ProgressSender,
) -> Result<()> {
    let vm_name = domain.name.clone();

    let _ = progress.send_status(format!("Creating VM: {vm_name}"), StatusKind::Progress);

    let rendered_xml = domain
        .render()
        .with_context(|| format!("Failed to render XML for VM: {vm_name}"))?;

    let conn_for_blocking = qemu_conn.clone();
    let vm_name_for_blocking = vm_name.clone();

    tokio::task::spawn_blocking(move || -> Result<()> {
        libvirt::create_vm(&conn_for_blocking, &rendered_xml)
            .with_context(|| format!("Failed to create VM: {vm_name_for_blocking}"))?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("Task join error: {e:?}"))??;

    let _ = progress.send_status(format!("Created VM: {vm_name}"), StatusKind::Done);

    Ok(())
}

// ============================================================================
// Container Startup
// ============================================================================

/// Start a container node with all its network attachments.
#[instrument(
    skip(docker, qemu, spec, progress),
    fields(container_name = %spec.name, image = %spec.image),
    level = "debug"
)]
pub async fn start_container_node(
    docker: &Arc<bollard::Docker>,
    qemu: &Arc<Qemu>,
    spec: ContainerSpec,
    model_kind: data::ModelKind,
    progress: &ProgressSender,
) -> Result<bool> {
    let container_name = spec.name.clone();
    let is_running = container::run_container(docker, spec).await?;

    if !is_running {
        return Ok(false);
    }
    finish_management_checksums(&KernelHost::new(docker, qemu), &container_name, model_kind)
        .await?;

    // SR Linux: flush default namespace IP to avoid DUP pings
    if model_kind.model() == data::NodeModel::NokiaSrlinux {
        let flush_cmd = "for i in $(seq 1 30); do ip netns list 2>/dev/null | grep -q srbase-mgmt && break; sleep 1; done; ip addr flush dev mgmt0";
        container::exec_container_detached(docker, &container_name, vec!["sh", "-c", flush_cmd])
            .await
            .with_context(|| {
                format!("Failed to flush default namespace IP for {container_name}")
            })?;
    }

    let _ = progress.send_status(format!("Node {container_name} - Started"), StatusKind::Done);

    Ok(true)
}

/// Turn transmit checksum offload off on the host end of a running container's management
/// veth, when it is on, for a model whose container needs finished checksums. Docker makes the
/// veth again, with offload on, at every start of the container, so this follows every start:
/// Sherpa's own, and through the scanner one outside Sherpa. Returns whether it turned it off.
#[instrument(skip(host), level = "debug")]
pub(crate) async fn finish_management_checksums(
    host: &impl LinkHost,
    container_name: &str,
    model_kind: data::ModelKind,
) -> Result<bool> {
    if !model_kind.needs_host_checksums() {
        return Ok(false);
    }
    let ifindex = host
        .management_veth(container_name)
        .await
        .with_context(|| format!("Failed to find the management veth of {container_name}"))?;
    let on = host.tx_checksum_offload_on(ifindex).with_context(|| {
        format!("Failed to read checksum offload on the management veth of {container_name}")
    })?;
    if !on {
        return Ok(false);
    }
    host.disable_tx_checksum_offload(ifindex).with_context(|| {
        format!(
            "Failed to turn transmit checksum offload off on the management veth of \
             {container_name}"
        )
    })?;
    tracing::info!(%container_name, ifindex, "Management veth checksum offload off");
    Ok(true)
}

// ============================================================================
// Node Readiness Check
// ============================================================================

/// Check if a node is ready by attempting a TCP connection to its readiness port.
pub fn check_node_port_ready(ip: &str, port: u16) -> Result<bool> {
    let address_port = format!("{ip}:{port}").parse()?;
    match TcpStream::connect_timeout(&address_port, Duration::from_millis(100)) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Check if a virtual machine is ready on its readiness port. On the SSH port an SSH server
/// must answer (`check_node_ssh_ready`), and its host key is returned; with `sshd_marker`, the
/// node's stored `ReadyCheck::sshd_marker`, set only on an SSH check, its identification line
/// must show [`SHERPA_SSHD_READY_MARKER`] too, which it does only once it runs with the config
/// Sherpa gave it. Any other port, such as the Talos API, needs a TCP connection.
pub async fn check_vm_ready(ip: &str, port: u16, sshd_marker: bool) -> Result<VmReadiness> {
    if port != SSH_PORT && !sshd_marker {
        return Ok(if check_node_port_ready(ip, port)? {
            VmReadiness::Ready { host_key: None }
        } else {
            VmReadiness::NotReady
        });
    }
    let Some(host_key) = check_node_ssh_ready(ip, port).await? else {
        return Ok(VmReadiness::NotReady);
    };
    if sshd_marker {
        let identification = ssh_identification(ip, port).await?;
        if !identification
            .as_deref()
            .is_some_and(|line| identification_shows(line, SHERPA_SSHD_READY_MARKER))
        {
            tracing::debug!(
                ?identification,
                "SSH server answers, but not yet with the config Sherpa gave it"
            );
            return Ok(VmReadiness::NotReady);
        }
    }
    Ok(VmReadiness::Ready {
        host_key: Some(host_key),
    })
}

/// The identification line the SSH server at `ip`:`port` sends as soon as it accepts a
/// connection, such as `SSH-2.0-OpenSSH_for_Windows_9.5 sherpa-ready`, before any key
/// exchange. `None` when it sends none within the handshake timeout. Nothing is sent to it.
#[instrument(level = "debug")]
pub async fn ssh_identification(ip: &str, port: u16) -> Result<Option<String>> {
    let address: SocketAddr = format!("{ip}:{port}").parse()?;
    match tokio::time::timeout(
        Duration::from_secs(SSH_READY_HANDSHAKE_TIMEOUT),
        read_ssh_identification(address),
    )
    .await
    {
        Ok(Ok(line)) => Ok(line),
        Ok(Err(error)) => {
            tracing::debug!(%error, "No SSH identification line");
            Ok(None)
        }
        Err(_) => {
            tracing::debug!("No SSH identification line in time");
            Ok(None)
        }
    }
}

/// Read lines from the SSH server at `address` until its identification line, the first that
/// starts with `SSH-`.
async fn read_ssh_identification(address: SocketAddr) -> io::Result<Option<String>> {
    let stream = tokio::net::TcpStream::connect(address).await?;
    let mut lines = BufReader::new(stream.take(SSH_IDENTIFICATION_MAX_BYTES)).lines();
    while let Some(line) = lines.next_line().await? {
        if line.starts_with("SSH-") {
            return Ok(Some(line));
        }
    }
    Ok(None)
}

/// Whether the SSH identification `line` shows `marker` among its comments, the words after
/// its protocol and software version, where OpenSSH puts its `VersionAddendum`.
fn identification_shows(line: &str, marker: &str) -> bool {
    line.split_once(' ')
        .is_some_and(|(_, comments)| comments.split_whitespace().any(|word| word == marker))
}

/// Check if an SSH server answers on a node: the port must accept a TCP connection and then
/// complete the SSH handshake. A port that is open before the SSH server is, such as a
/// network device's or a port forward with nothing behind it, is not ready.
///
/// Returns the host key the node offered, in OpenSSH form, once it answers. No credentials
/// are sent. The key is accepted without checking, as this is the first time the server
/// sees the node, and it is recorded for the lab's `known_hosts` file. Older algorithms are
/// allowed, as some network devices offer nothing else.
#[instrument(level = "debug")]
pub async fn check_node_ssh_ready(ip: &str, port: u16) -> Result<Option<String>> {
    if !check_node_port_ready(ip, port)? {
        return Ok(None);
    }
    let hop = Hop::new(
        Node::new(ip, port),
        Auth::new(SHERPA_USERNAME),
        HostKeyPolicy::Insecure,
    )
    .allow_legacy_algorithms();
    let mut options = ConnectOptions::default();
    options.connect_timeout = Duration::from_secs(SSH_READY_HANDSHAKE_TIMEOUT);
    options.keepalive_interval = None;
    match Session::host_key(&hop, &options).await {
        Ok(host_key) => Ok(Some(host_key)),
        Err(error) => {
            tracing::debug!(%error, "SSH server not ready");
            Ok(None)
        }
    }
}

/// Port that shows a container is ready once it has kept running: the manifest's
/// `ready_check.port`, else its model and kind's port, which only the `sonic_linux` container has.
/// Without one it is ready once it keeps running; a skipped check has none either.
pub fn container_ready_port(
    model_kind: data::ModelKind,
    ready_check: &NodeReadyCheck,
) -> Option<u16> {
    if ready_check.skip.unwrap_or(false) {
        None
    } else {
        ready_check.port.or(model_kind.ready_port())
    }
}

/// The readiness check stored with a node, from its manifest `ready_check` and its effective
/// `ztp`: the manifest's port, else its model and kind's port, and, when that is SSH, whether
/// its SSH server must show Sherpa's ready marker (`ModelKind::waits_for_sshd_ready_marker`);
/// a Windows node checked on another port is checked as that port. A virtual machine and the
/// `sonic_linux` container have a model port; any other container or a unikernel without a manifest
/// port has none and is ready once it runs. Every readiness probe, in `up`, `redeploy`, `resume`, the scanner and
/// `node.host_keys`, reads this check.
pub fn stored_ready_check(
    model_kind: data::ModelKind,
    ready_check: &NodeReadyCheck,
    ztp: data::Ztp,
) -> data::ReadyCheck {
    let port = ready_check.port.or(model_kind.ready_port());
    data::ReadyCheck {
        port,
        skip: ready_check.skip.unwrap_or(false),
        sshd_marker: port == Some(SSH_PORT) && model_kind.waits_for_sshd_ready_marker(ztp),
    }
}

/// The port a node's stored readiness check is probed on. `None` when the check is skipped or
/// has no port: the node is ready once it runs.
pub fn probed_ready_port(ready_check: &data::ReadyCheck) -> Option<u16> {
    if ready_check.skip {
        None
    } else {
        ready_check.port
    }
}

/// Human-readable name of a readiness port, for status messages.
pub fn ready_port_label(port: u16) -> String {
    match port {
        SSH_PORT => "SSH".to_owned(),
        TALOS_API_PORT => "Talos API".to_owned(),
        _ => format!("port {port}"),
    }
}

// ============================================================================
// Unikernel Operations
// ============================================================================

pub struct UnikernelSetupResult {
    pub clone_disks: Vec<data::CloneDisk>,
    pub disks: Vec<data::NodeDisk>,
    pub kernel_path: Option<String>,
    pub mac_address: String,
}

/// Prepare unikernel disk/kernel paths for deployment.
///
/// For `DiskBoot` mode: clones the disk image like a VM.
/// For `DirectKernel` mode: clones the kernel ELF to the storage pool.
#[instrument(skip(node_image), fields(node_name = %node.name, boot_mode), level = "debug")]
pub fn generate_unikernel_setup(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_id: &str,
    images_dir: &str,
) -> Result<UnikernelSetupResult> {
    let unikernel = unikernel_settings(node_image)?;
    let boot_mode = unikernel.boot_mode.clone();

    let mac_address = util::random_mac(KVM_OUI);
    let version = node.version.as_deref().unwrap_or(&node_image.version);
    let filename = match boot_mode {
        data::UnikernelBootMode::DiskBoot => UNIKERNEL_DISK_FILENAME,
        data::UnikernelBootMode::DirectKernel => UNIKERNEL_KERNEL_FILENAME,
    };

    match boot_mode {
        data::UnikernelBootMode::DiskBoot => {
            let src_disk = format!(
                "{}/{}/{}/{}",
                images_dir, node_image.model, version, filename
            );
            let dst_disk = format!(
                "{}/{}-{}.qcow2",
                SHERPA_STORAGE_POOL_PATH, node.name, lab_id
            );

            let clone_disk = data::CloneDisk {
                src: src_disk,
                dst: dst_disk.clone(),
                disk_size: node.boot_disk.size,
                method: data::DiskImageCloneMethod::Copy,
            };

            let disk = data::NodeDisk {
                driver_name: data::DiskDrivers::Qemu,
                driver_format: data::DiskFormats::Qcow2,
                src_file: dst_disk,
                target_dev: data::DiskTargets::Vda,
                target_bus: data::DiskBuses::Virtio,
                disk_device: data::DiskDevices::File,
            };

            Ok(UnikernelSetupResult {
                clone_disks: vec![clone_disk],
                disks: vec![disk],
                kernel_path: None,
                mac_address,
            })
        }
        data::UnikernelBootMode::DirectKernel => {
            let src_kernel = format!(
                "{}/{}/{}/{}",
                images_dir, node_image.model, version, filename
            );
            let dst_kernel = format!("{}/{}-{}.elf", SHERPA_STORAGE_POOL_PATH, node.name, lab_id);

            let clone_disk = data::CloneDisk {
                src: src_kernel,
                dst: dst_kernel.clone(),
                disk_size: None,
                method: data::DiskImageCloneMethod::Copy,
            };

            Ok(UnikernelSetupResult {
                clone_disks: vec![clone_disk],
                disks: vec![],
                kernel_path: Some(dst_kernel),
                mac_address,
            })
        }
    }
}

/// Build a unikernel domain template with auto IP injection for `DirectKernel` models.
///
/// # Errors
///
/// Fails when `node_image` is not a unikernel image.
#[instrument(skip(node_image, parts), fields(node_name = %node.name), level = "debug")]
pub fn build_unikernel_domain_template(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_id: &str,
    qemu_bin: &str,
    parts: DomainParts,
    kernel_path: Option<String>,
) -> Result<template::UnikernelDomainTemplate> {
    let node_name_with_lab = format!("{}-{}", node.name, lab_id);
    let unikernel = unikernel_settings(node_image)?;

    // Only the manifest sets a kernel cmdline: Unikraft's pre-built images take their address
    // by DHCP, from the static MAC-to-IP binding on the Sherpa router, and treat the cmdline as
    // the application path.
    let kernel_cmdline = node
        .unikernel
        .as_ref()
        .and_then(|unikernel| unikernel.cmdline.clone());

    Ok(template::UnikernelDomainTemplate {
        qemu_bin: qemu_bin.to_string(),
        name: node_name_with_lab,
        memory: node.memory.unwrap_or(node_image.memory),
        cpu_architecture: unikernel.cpu.architecture.clone(),
        cpu_model: unikernel.cpu.model.clone(),
        machine_type: unikernel.machine.kind.clone(),
        cpu_count: node.cpu.count.unwrap_or(node_image.cpu.count),
        boot_mode: unikernel.boot_mode.clone(),
        kernel_path,
        kernel_cmdline,
        disks: parts.disks,
        interfaces: parts.interfaces,
        interface_type: unikernel.interfaces.kind.clone(),
        management_interface_type: unikernel.interfaces.kind.clone(),
        reserved_interface_type: unikernel.interfaces.kind.clone(),
        serial_socket: parts.consoles.serial,
        management_network: parts.management_network,
        isolated_network: parts.isolated_network,
        reserved_network: parts.reserved_network,
    })
}

/// The `vm` group of a virtual machine's image.
///
/// # Errors
///
/// Fails when `node_image` is another kind's image.
fn vm_settings(node_image: &data::NodeConfig) -> Result<&data::VmSettings> {
    node_image.vm().with_context(|| {
        format!(
            "Node image {} {} is a {} image, not a virtual machine's",
            node_image.model,
            node_image.version,
            node_image.kind()
        )
    })
}

/// The `unikernel` group of a unikernel's image.
///
/// # Errors
///
/// Fails when `node_image` is another kind's image.
fn unikernel_settings(node_image: &data::NodeConfig) -> Result<&data::UnikernelSettings> {
    node_image.unikernel().with_context(|| {
        format!(
            "Node image {} {} is a {} image, not a unikernel's",
            node_image.model,
            node_image.version,
            node_image.kind()
        )
    })
}

/// Create a unikernel from a domain template using libvirt.
#[instrument(skip(qemu_conn, domain, progress), fields(uk_name = %domain.name), level = "debug")]
pub async fn create_unikernel(
    qemu_conn: Arc<libvirt::QemuConnection>,
    domain: template::UnikernelDomainTemplate,
    progress: &ProgressSender,
) -> Result<()> {
    let uk_name = domain.name.clone();

    let _ = progress.send_status(
        format!("Creating unikernel: {uk_name}"),
        StatusKind::Progress,
    );

    let rendered_xml = domain
        .render()
        .with_context(|| format!("Failed to render XML for unikernel: {uk_name}"))?;

    let conn_for_blocking = qemu_conn.clone();
    let uk_name_for_blocking = uk_name.clone();

    tokio::task::spawn_blocking(move || -> Result<()> {
        libvirt::create_vm(&conn_for_blocking, &rendered_xml)
            .with_context(|| format!("Failed to create unikernel: {uk_name_for_blocking}"))?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("Task join error: {e:?}"))??;

    let _ = progress.send_status(format!("Created unikernel: {uk_name}"), StatusKind::Done);

    Ok(())
}

/// Check if a unikernel is ready by verifying libvirt domain state, with optional TCP port probe.
#[instrument(skip(qemu_conn), level = "debug")]
pub fn check_unikernel_ready(
    qemu_conn: &libvirt::QemuConnection,
    domain_name: &str,
    ready_port: Option<u16>,
    mgmt_ip: Option<&str>,
) -> Result<bool> {
    // Check if domain is running via libvirt
    let Ok(domain) = virt::domain::Domain::lookup_by_name(qemu_conn, domain_name) else {
        return Ok(false);
    };

    let (state, _) = domain.get_state()?;
    if state != virt::sys::VIR_DOMAIN_RUNNING {
        return Ok(false);
    }

    // If ready_port specified, also do TCP probe
    if let (Some(port), Some(ip)) = (ready_port, mgmt_ip) {
        check_node_port_ready(ip, port)
    } else {
        // Domain running = ready (default for unikernels)
        Ok(true)
    }
}

// ============================================================================
// ============================================================================
// Node Destruction
// ============================================================================

/// Destroy a single VM node: destroy domain, undefine, delete disks, and destroy networks.
#[instrument(skip(qemu_conn), fields(%node_name, %lab_id), level = "debug")]
pub async fn destroy_vm_node(
    qemu_conn: Arc<libvirt::QemuConnection>,
    node_name: &str,
    lab_id: &str,
    node_idx: u16,
    reserved_interface_count: u8,
) -> Result<()> {
    let node_name_with_lab = format!("{node_name}-{lab_id}");

    // Destroy and undefine VM domain
    let conn = qemu_conn.clone();
    let vm_name = node_name_with_lab.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        match virt::domain::Domain::lookup_by_name(&conn, &vm_name) {
            Ok(domain) => {
                if domain.is_active()? {
                    domain.destroy()?;
                }
                domain.undefine_flags(VIR_DOMAIN_UNDEFINE_NVRAM)?;
                tracing::info!(vm_name = %vm_name, "VM destroyed and undefined");
            }
            Err(error) if error.code() == ErrorNumber::NoDomain => {}
            Err(error) => {
                return Err(error).context("Cannot determine VM existence; refusing disk removal");
            }
        }

        Ok(())
    })
    .await
    .map_err(|e| anyhow!("Task join error: {e:?}"))??;

    // Delete VM disks from storage pool
    let conn = qemu_conn.clone();
    let disk_prefix = node_name_with_lab;
    tokio::task::spawn_blocking(move || -> Result<()> {
        let pool = virt::storage_pool::StoragePool::lookup_by_name(&conn, SHERPA_STORAGE_POOL)
            .context("Cannot find storage pool during node cleanup")?;
        libvirt::refresh_pool(&pool).context("Cannot refresh storage pool during node cleanup")?;
        for vol in pool
            .list_all_volumes(0)
            .context("Cannot list volumes during node cleanup")?
        {
            let vol_name = vol
                .get_name()
                .context("Cannot identify volume during node cleanup")?;
            if vol_name.starts_with(&disk_prefix) {
                vol.delete(0).with_context(|| {
                    format!(
                        "Cannot delete node disk {vol_name}; retaining image reference for retry"
                    )
                })?;
            }
        }

        Ok(())
    })
    .await
    .map_err(|e| anyhow!("Task join error: {e:?}"))??;

    // Destroy per-node isolated network
    let isolated = node_isolated_network_data(node_name, node_idx, lab_id);
    let conn = qemu_conn.clone();
    let iso_net_name = isolated.network_name.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        if let Ok(net) = virt::network::Network::lookup_by_name(&conn, &iso_net_name) {
            if net.is_active()? {
                net.destroy()?;
            }
            net.undefine()?;
            tracing::info!(network = %iso_net_name, "Destroyed isolated network");
        }
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("Task join error: {e:?}"))??;

    // Destroy per-node reserved network if applicable
    if reserved_interface_count > 0 {
        let reserved = node_reserved_network_data(node_name, node_idx, lab_id);
        let conn = qemu_conn.clone();
        let res_net_name = reserved.network_name.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Ok(net) = virt::network::Network::lookup_by_name(&conn, &res_net_name) {
                if net.is_active()? {
                    net.destroy()?;
                }
                net.undefine()?;
                tracing::info!(network = %res_net_name, "Destroyed reserved network");
            }
            Ok(())
        })
        .await
        .map_err(|e| anyhow!("Task join error: {e:?}"))??;
    }

    Ok(())
}

/// Destroy a single container node: kill, remove, and delete its Docker data networks.
///
/// # Errors
///
/// Fails when the container or one of its networks cannot be removed. Each step is
/// idempotent, so a failed redeploy can run it again.
#[instrument(skip(docker), fields(%node_name, %lab_id), level = "debug")]
pub async fn destroy_container_node(
    docker: &bollard::Docker,
    node_name: &str,
    lab_id: &str,
) -> Result<()> {
    let container_name = format!("{node_name}-{lab_id}");
    // Killing fails when the container is not running, which is fine: it is removed next.
    if let Err(error) = container::kill_container(docker, &container_name).await {
        tracing::debug!(container = %container_name, error = %error, "Container not killed");
    }
    container::remove_container(docker, &container_name)
        .await
        .with_context(|| format!("Failed to remove container {container_name}"))?;

    let networks = container::list_networks(docker)
        .await
        .context("Failed to list Docker networks")?;
    for net_name in networks.into_iter().filter_map(|net| net.name) {
        if is_node_network(&net_name, node_name, lab_id) {
            container::delete_network(docker, &net_name)
                .await
                .with_context(|| format!("Failed to delete Docker network {net_name}"))?;
            tracing::info!(network = %net_name, "Destroyed Docker network for node");
        }
    }

    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::konst::TFTP_DIR_MODE;
    use crate::log_capture::capture_log;
    use crate::services::guest_account::tests::{RSA_4096_KEY, RSA_8192_KEY, test_account};
    use axum::extract::ws::Message;
    use nix::sys::stat::{Mode, umask};
    use shared::data::{ModelKind, Ztp};
    use std::env;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use tempfile::tempdir;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::mpsc;

    #[test]
    fn boot_files_012_on_flatcar_a_systemd_unit_has_its_contents_and_enabled_state() {
        let units = ignition_units(&[SystemdUnitData {
            name: "app.service".to_owned(),
            content: util::base64_encode("[Unit]\nDescription=app\n"),
            enabled: false,
        }])
        .expect("units");
        assert_eq!(units[0].name, "app.service");
        assert_eq!(units[0].enabled, Some(false));
        assert_eq!(
            units[0].contents.as_deref(),
            Some("[Unit]\nDescription=app\n")
        );
    }

    #[test]
    fn node_interfaces_012_only_a_model_that_waits_for_its_interfaces_has_unused_ones_up() {
        assert_eq!(
            netns_veth_state(
                ModelKind::new(
                    data::NodeModel::JuniperEvolved,
                    Some(data::NodeKind::Container)
                )
                .unwrap(),
                false
            ),
            NetnsVethState::Enabled
        );
        assert_eq!(
            netns_veth_state(
                ModelKind::new(
                    data::NodeModel::JuniperEvolved,
                    Some(data::NodeKind::Container)
                )
                .unwrap(),
                true
            ),
            NetnsVethState::DisabledUp
        );
        assert_eq!(
            netns_veth_state(
                ModelKind::new(data::NodeModel::AristaEos, Some(data::NodeKind::Container))
                    .unwrap(),
                true
            ),
            NetnsVethState::Disabled
        );
    }

    #[test]
    fn container_run_003_a_variable_the_user_sets_replaces_sherpas_of_the_same_key() {
        let merged = merge_env_vars(
            vec![
                "KC_DB=dev-file".to_owned(),
                "KC_HEALTH_ENABLED=true".to_owned(),
            ],
            &["KC_DB=postgres".to_owned(), "KC_LOG_LEVEL=debug".to_owned()],
        );
        assert_eq!(
            merged,
            [
                "KC_HEALTH_ENABLED=true",
                "KC_DB=postgres",
                "KC_LOG_LEVEL=debug"
            ]
        );
    }

    #[test]
    fn keycloak_serves_https_only_with_the_lab_certificate_and_its_own_admin() {
        let admin = data::AdminCredential {
            username: "admin".to_owned(),
            password: "kc01-admin-pw".to_owned(),
        };
        let env = keycloak_env_vars(Ipv4Addr::new(172, 31, 1, 22), &admin);
        for expected in [
            "KC_DB=dev-file",
            "KC_HTTP_ENABLED=false",
            "KC_HOSTNAME=https://172.31.1.22:8443",
            "KC_HTTPS_CERTIFICATE_FILE=/etc/sherpa/certs/node.crt",
            "KC_HTTPS_CERTIFICATE_KEY_FILE=/etc/sherpa/certs/node.key",
            "KC_BOOTSTRAP_ADMIN_USERNAME=admin",
            "KC_BOOTSTRAP_ADMIN_PASSWORD=kc01-admin-pw",
        ] {
            assert!(env.iter().any(|var| var == expected), "missing {expected}");
        }
    }

    #[test]
    fn vault_cli_reaches_the_api_over_tls_and_trusts_the_lab_ca() {
        assert_eq!(
            vault_env_vars(Ipv4Addr::new(172, 31, 1, 21)),
            [
                "VAULT_ADDR=https://172.31.1.21:8200",
                "VAULT_CACERT=/etc/sherpa/certs/ca.crt"
            ]
        );
    }

    #[test]
    fn cevolved_env_selects_the_platform_and_sizes_the_evo_vm() {
        assert_eq!(
            cevolved_env_vars(data::PtxPlatform::Bt, 12, 4, 6292),
            [
                "CPTX_COSIM=BT",
                "CPTX_NUM_IF=12",
                "CPTX_EVOVM_CPU=4",
                "CPTX_EVOVM_MEM_MB=6292"
            ]
        );
        assert_eq!(
            cevolved_env_vars(data::PtxPlatform::Bx, 36, 8, 16384)[..2],
            ["CPTX_COSIM=BX", "CPTX_NUM_IF=36"]
        );
    }

    #[test]
    fn login_environment_002_the_file_exports_each_variable_and_keeps_a_single_quote() {
        let env_vars = vec!["EDITOR=vim".to_string(), "MSG=it's a test".to_string()];
        let script = build_env_profile_script(&env_vars);
        let home = tempdir().unwrap();
        let file = home.path().join("env.sh");
        std::fs::write(&file, script).unwrap();
        let output = Command::new("sh")
            .arg("-c")
            .arg(r#". "$1"; sh -c 'printf "%s|%s" "$EDITOR" "$MSG"'"#)
            .arg("sh")
            .arg(&file)
            .env_remove("EDITOR")
            .env_remove("MSG")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "vim|it's a test");
    }

    #[test]
    fn login_environment_003_the_file_is_sherpa_env_owner_only_and_loads_the_variable() {
        let file = build_user_env_file(&["GH_TOKEN=abc123".to_string()], "alice");
        assert_eq!(
            file.dst, "~/.sherpa_env",
            "home differs by OS, /usr/home on FreeBSD"
        );
        assert_eq!(file.user, "alice");
        assert_eq!(
            file.group, None,
            "uses the primary group, which varies by distro"
        );
        assert_eq!(file.permissions, "600");
        assert_eq!(file.encoding, None);
        let home = tempdir().unwrap();
        std::fs::write(home.path().join(".sherpa_env"), &file.content).unwrap();
        assert_eq!(
            login_shell_var(home.path(), r#". "$HOME/.sherpa_env""#, "GH_TOKEN"),
            "abc123"
        );
    }

    /// A fake passwd file listing the current user with `home`, so generated commands can run
    /// without touching the real home directory.
    fn fake_passwd(root: &std::path::Path, home: &std::path::Path) -> (String, String) {
        let output = std::process::Command::new("id")
            .arg("-un")
            .output()
            .unwrap();
        let user = String::from_utf8(output.stdout).unwrap().trim().to_string();
        let passwd = root.join("passwd");
        std::fs::write(
            &passwd,
            format!("{user}:x:1000:1000::{}:/bin/sh\n", home.display()),
        )
        .unwrap();
        (user, passwd.to_string_lossy().into_owned())
    }

    fn run_sh(commands: &[String]) -> bool {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(commands.join("\n"))
            .status()
            .unwrap()
            .success()
    }

    /// The value of `var` after `sh` runs `script` with `HOME` set to `home`.
    fn login_shell_var(home: &Path, script: &str, var: &str) -> String {
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!(
                r#"{script}
printf %s "${var}""#
            ))
            .env("HOME", home)
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap()
    }

    /// Runs `commands` under `sh` with a `chown` that only records its arguments, one call per
    /// line, and returns the record.
    fn chown_calls(root: &Path, commands: &[String]) -> String {
        let bin = root.join("bin");
        let log = root.join("chown.log");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(&log, "").unwrap();
        let chown = bin.join("chown");
        std::fs::write(
            &chown,
            format!(
                r#"#!/bin/sh
echo "$*" >> '{}'
"#,
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&chown, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:{}", bin.display(), env::var("PATH").unwrap());
        Command::new("sh")
            .arg("-c")
            .arg(commands.join("\n"))
            .env("PATH", path)
            .status()
            .unwrap();
        std::fs::read_to_string(&log).unwrap()
    }

    #[test]
    fn login_environment_004_the_loader_hooks_the_login_file_bash_reads_in_the_passwd_home() {
        let root = tempdir().unwrap();
        let home = root.path().join("usr/home/alice");
        std::fs::create_dir_all(&home).unwrap();
        let (user, passwd) = fake_passwd(root.path(), &home);

        std::fs::write(home.join(".sherpa_env"), "export GH_TOKEN='abc123'\n").unwrap();

        assert!(run_sh(&[user_env_loader_runcmd(&user, &passwd)]));
        assert_eq!(
            login_shell_var(&home, r#". "$HOME/.profile""#, "GH_TOKEN"),
            "abc123"
        );

        std::fs::write(home.join(".bash_profile"), "").unwrap();
        assert!(run_sh(&[user_env_loader_runcmd(&user, &passwd)]));
        assert_eq!(
            login_shell_var(&home, r#". "$HOME/.bash_profile""#, "GH_TOKEN"),
            "abc123"
        );
    }

    #[test]
    fn login_environment_005_an_unknown_user_gets_no_login_file() {
        let root = tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let (_, passwd) = fake_passwd(root.path(), &home);
        let calls = chown_calls(
            root.path(),
            &[user_env_loader_runcmd("nosuchuser", &passwd)],
        );
        assert_eq!(calls, "");
        assert!(!home.join(".profile").exists());
        assert!(!home.join(".bash_profile").exists());
    }

    fn staged(dst: &str, user: &str, group: Option<&str>) -> StagedFile {
        StagedFile {
            dst: dst.to_string(),
            content: "secret123".to_string(),
            encoding: None,
            user: user.to_string(),
            group: group.map(str::to_string),
            permissions: "600".to_string(),
        }
    }

    #[test]
    fn boot_files_003_cloud_init_gets_each_text_file_root_only_outside_its_destination() {
        let files = vec![staged("/home/alice/.sherpa_env", "alice", None)];
        let (writes, runcmd) = stage_cloud_init_files(&files, "/opt/sherpa/staging", "/etc/passwd");
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path, "/opt/sherpa/staging/0");
        assert_eq!(
            writes[0].owner, None,
            "the distro default owner, root:wheel on BSD"
        );
        assert_eq!(writes[0].permissions, "0600");
        assert_eq!(writes[0].content, "secret123");
        assert_eq!(runcmd.len(), 2);
        assert!(
            !runcmd.concat().contains("secret123"),
            "values stay out of runcmd"
        );
        assert_eq!(runcmd[1], "rm -rf '/opt/sherpa/staging'");
    }

    #[test]
    fn boot_files_007_a_node_with_no_text_files_or_variables_gets_no_file_or_command() {
        let (writes, runcmd) = stage_cloud_init_files(&[], "/opt/sherpa/staging", "/etc/passwd");
        assert!(writes.is_empty());
        assert_eq!(runcmd, [] as [std::string::String; 0]);
    }

    /// Runs the generated commands under `sh` to check quoting, home lookup, parent directories,
    /// mode, and removal of the staged copies.
    #[test]
    fn boot_files_004_each_file_is_moved_to_its_destination_and_nothing_staged_is_left() {
        let root = tempdir().unwrap();
        let staging = root.path().join("staging");
        let home = root.path().join("usr/home/alice");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let (user, passwd) = fake_passwd(root.path(), &home);

        let nested = home.join("it's dir/.claude/.credentials.json");
        let outside = root.path().join("opt/app/config.toml");
        let files = vec![
            staged(nested.to_str().unwrap(), &user, None),
            staged("~/.sherpa_env", &user, None),
            staged(outside.to_str().unwrap(), &user, None),
        ];
        let (writes, runcmd) = stage_cloud_init_files(&files, staging.to_str().unwrap(), &passwd);
        for write in &writes {
            std::fs::write(&write.path, &write.content).unwrap();
        }

        assert!(run_sh(&runcmd));
        assert_eq!(std::fs::read_to_string(&nested).unwrap(), "secret123");
        assert_eq!(
            std::fs::read_to_string(home.join(".sherpa_env")).unwrap(),
            "secret123",
            "~/ resolves to the passwd home"
        );
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "secret123");
        let mode = std::fs::metadata(&nested).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(!staging.exists(), "staging directory is removed");
    }

    #[test]
    fn boot_files_005_a_missing_folder_is_given_to_the_owner_only_inside_their_home() {
        let root = tempdir().unwrap();
        let staging = root.path().join("staging");
        let home = root.path().join("home/alice");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let (user, passwd) = fake_passwd(root.path(), &home);
        let inside = home.join(".config/app/app.toml");
        let outside = root.path().join("opt/app/config.toml");
        let files = vec![
            staged(inside.to_str().unwrap(), &user, Some("staff")),
            staged(outside.to_str().unwrap(), &user, Some("staff")),
        ];
        let (writes, runcmd) = stage_cloud_init_files(&files, staging.to_str().unwrap(), &passwd);
        for write in &writes {
            std::fs::write(&write.path, &write.content).unwrap();
        }

        let calls = chown_calls(root.path(), &runcmd);

        let expected = [
            format!("-R {user}:staff {}", home.join(".config").display()),
            format!("{user}:staff {}", inside.display()),
            format!("{user}:staff {}", outside.display()),
        ];
        assert_eq!(calls.lines().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn boot_files_006_a_file_for_a_user_the_node_lacks_is_not_written() {
        let root = tempdir().unwrap();
        let staging = root.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let (_, passwd) = fake_passwd(root.path(), root.path());
        let dst = root.path().join("opt/app/config.toml");
        let files = vec![staged(dst.to_str().unwrap(), "nosuchuser", None)];
        let (writes, runcmd) = stage_cloud_init_files(&files, staging.to_str().unwrap(), &passwd);
        for write in &writes {
            std::fs::write(&write.path, &write.content).unwrap();
        }

        let calls = chown_calls(root.path(), &runcmd);

        assert_eq!(calls, "");
        assert!(!dst.exists());
    }

    #[test]
    fn login_environment_006_on_flatcar_the_file_is_private_and_appended_to_bash_profile() {
        let files = build_ignition_user_env(&["GH_TOKEN=abc123".to_string()], "alice");
        assert_eq!(files.len(), 2);

        let env_file = &files[0];
        assert_eq!(env_file.path, "/home/alice/.sherpa_env");
        assert_eq!(env_file.mode, template::FileMode::new(0o600));
        assert_eq!(env_file.user.as_ref().unwrap().name, "alice");
        assert_eq!(env_file.group.as_ref().unwrap().name, "alice");
        let source = &env_file.contents.as_ref().unwrap().source;
        let encoded = source.strip_prefix("data:;base64,").unwrap();
        let home = tempdir().unwrap();
        std::fs::write(
            home.path().join(".sherpa_env"),
            util::base64_decode(encoded).unwrap(),
        )
        .unwrap();

        let profile = &files[1];
        assert_eq!(profile.path, "/home/alice/.bash_profile");
        assert!(profile.contents.is_none(), "append keeps the skeleton file");
        assert_eq!(profile.user.as_ref().unwrap().name, "alice");
        let encoded = profile.append[0]
            .source
            .strip_prefix("data:;base64,")
            .unwrap();
        let appended = util::base64_decode(encoded).unwrap();
        assert_eq!(
            login_shell_var(home.path(), &appended, "GH_TOKEN"),
            "abc123"
        );
    }

    #[test]
    fn config_delivery_029_ignition_certificates_are_0644_and_the_key_0600() {
        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for name in ["ca.crt", "node.crt", "node.key"] {
            let file = dir.path().join(name);
            std::fs::write(&file, "pem").unwrap();
            paths.push(file.to_string_lossy().into_owned());
        }
        let certs = NodeCertPaths {
            ca_cert: paths[0].clone(),
            node_cert: paths[1].clone(),
            node_key: paths[2].clone(),
        };
        let mut modes = Vec::new();
        for file in build_ignition_cert_files(&certs, NODE_CERTS_DIR).unwrap() {
            modes.push(serde_json::to_value(&file).unwrap()["mode"].clone());
        }
        assert_eq!(modes, vec![0o644, 0o644, 0o600]);
    }

    fn script(filename: &str) -> topology::StartupScript {
        topology::StartupScript {
            filename: filename.to_string(),
            content: "IyEvYmluL3NoCmVjaG8gaGk=".to_string(),
        }
    }

    #[test]
    fn boot_scripts_002_startup_scripts_are_written_executable_and_run_directly_in_order() {
        let (files, runcmd) =
            build_cloud_init_scripts("node1", Some(&[script("boot.sh"), script("next.sh")]))
                .unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "/opt/sherpa/startup_scripts/boot.sh");
        assert_eq!(files[0].content, "#!/bin/sh\necho hi");
        assert_eq!(files[0].permissions, "0755");
        assert_eq!(files[1].path, "/opt/sherpa/startup_scripts/next.sh");
        assert_eq!(
            runcmd,
            vec![
                r"'/opt/sherpa/startup_scripts/boot.sh'".to_string(),
                r"'/opt/sherpa/startup_scripts/next.sh'".to_string(),
            ]
        );
    }

    fn text_file(dst: &str, user: &str) -> topology::TextFileData {
        topology::TextFileData {
            content: "aGVsbG8=".to_string(),
            dst: dst.to_string(),
            user: user.to_string(),
            group: user.to_string(),
            permissions: 600,
        }
    }

    #[test]
    fn boot_files_010_on_flatcar_the_missing_folders_in_the_owners_home_are_theirs() {
        let text_files = Some(vec![
            text_file("/home/alice/.config/probe/nested/a.txt", "alice"),
            text_file("/home/alice/.config/probe/b.txt", "alice"),
            text_file("/home/alice/top.txt", "alice"),
            text_file("/etc/app/config.toml", "root"),
            text_file("/opt/app/config.toml", "alice"),
        ]);
        let dirs = build_ignition_home_directories(text_files.as_deref());
        let mut paths = Vec::new();
        for dir in &dirs {
            assert_eq!(dir.user.as_ref().unwrap().name, "alice");
            assert_eq!(dir.group.as_ref().unwrap().name, "alice");
            assert_eq!(dir.mode, template::FileMode::new(0o755));
            paths.push(dir.path.as_str());
        }
        assert_eq!(
            paths,
            vec![
                "/home/alice/.config",
                "/home/alice/.config/probe",
                "/home/alice/.config/probe/nested",
            ]
        );
    }

    /// The PowerShell command a `powershell -NoProfile -EncodedCommand` line runs: its base64
    /// argument decoded as UTF-16LE.
    fn encoded_powershell(line: &str) -> String {
        let encoded = line
            .strip_prefix("powershell -NoProfile -EncodedCommand ")
            .unwrap_or_else(|| panic!("not an encoded PowerShell command: {line}"));
        let bytes = util::base64_decode_bytes(encoded).unwrap();
        let (pairs, rest) = bytes.as_chunks::<2>();
        assert!(rest.is_empty(), "an odd number of UTF-16LE bytes: {line}");
        let units: Vec<u16> = pairs.iter().copied().map(u16::from_le_bytes).collect();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn login_environment_007_on_windows_each_variable_is_set_machine_wide_quotes_kept() {
        let env_vars = vec!["EDITOR=vim".to_string(), "MSG=it's here".to_string()];
        let commands: Vec<String> = build_windows_env_commands(&env_vars)
            .iter()
            .map(String::as_str)
            .map(encoded_powershell)
            .collect();
        assert_eq!(
            commands,
            [
                "[System.Environment]::SetEnvironmentVariable('EDITOR', 'vim', 'Machine')",
                "[System.Environment]::SetEnvironmentVariable('MSG', 'it''s here', 'Machine')",
            ]
        );
    }

    #[test]
    fn login_environment_008_on_windows_cmd_sees_no_percent_or_quote_of_a_value() {
        let env_vars = vec![r#"TOKEN=%PATH% "a" & calc & 'b'"#.to_string()];
        let commands = build_windows_env_commands(&env_vars);
        assert_eq!(commands.len(), 1);
        assert!(
            !commands[0].contains(['%', '"', '\'', '&']),
            "{}",
            commands[0]
        );
        assert_eq!(
            encoded_powershell(&commands[0]),
            r#"[System.Environment]::SetEnvironmentVariable('TOKEN', '%PATH% "a" & calc & ''b''', 'Machine')"#
        );
    }

    #[test]
    fn login_environment_009_on_windows_a_key_and_value_round_trip_through_the_encoding() {
        let env_vars = vec!["GREETING_1=café ☕ 日本 =x".to_string()];
        let commands = build_windows_env_commands(&env_vars);
        assert_eq!(
            encoded_powershell(&commands[0]),
            "[System.Environment]::SetEnvironmentVariable('GREETING_1', 'café ☕ 日本 =x', 'Machine')"
        );
    }

    fn cloudbase_file(path: &str, permissions: &str) -> template::CloudbaseWriteFile {
        template::CloudbaseWriteFile {
            path: path.to_string(),
            content: "content".to_string(),
            permissions: permissions.to_string(),
        }
    }

    #[test]
    fn boot_files_008_on_windows_only_a_file_others_cannot_read_is_limited_to_admins() {
        let files = vec![
            cloudbase_file(r"C:\sherpa\certs\ca.crt", "0644"),
            cloudbase_file(r"C:\sherpa\certs\node.key", "0600"),
            cloudbase_file(r"C:\app\shared.txt", "0640"),
            cloudbase_file(r"C:\sherpa\startup_scripts\run.ps1", "0755"),
        ];

        let commands = build_windows_private_file_acls(&files).unwrap();

        assert_eq!(
            commands,
            vec![
                r#"icacls "C:\sherpa\certs\node.key" /inheritance:r /grant *S-1-5-32-544:F /grant *S-1-5-18:F"#,
                r#"icacls "C:\app\shared.txt" /inheritance:r /grant *S-1-5-32-544:F /grant *S-1-5-18:F"#,
            ]
        );
    }

    #[test]
    fn boot_files_009_on_windows_a_path_with_a_quote_or_percent_is_refused() {
        for path in [r#"C:\app\a" & del C:\x & ".txt"#, r"C:\app\%TEMP%.txt"] {
            let files = vec![cloudbase_file(path, "0600")];
            assert!(build_windows_private_file_acls(&files).is_err(), "{path}");
        }
    }

    #[test]
    fn test_omarchy_uses_standard_vm_domain() {
        let config = data::NodeConfig::omarchy_linux();
        let node = topology::NodeExpanded {
            name: "desktop".to_owned(),
            model_kind: ModelKind::default_for(config.model),
            ..Default::default()
        };
        let domain = build_domain_template(
            &node,
            &config,
            "lab1",
            "/usr/bin/qemu-system-x86_64",
            DomainParts {
                disks: vec![],
                interfaces: vec![],
                consoles: ConsoleSockets {
                    serial: "/opt/sherpa/run/console/lab1/1.serial".to_owned(),
                    vnc: "/opt/sherpa/run/console/lab1/1.vnc".to_owned(),
                },
                management_network: "mgmt".to_owned(),
                isolated_network: String::new(),
                reserved_network: String::new(),
            },
            VmFirmware::default(),
            &data::NodeResources {
                cpu_count: Some(2),
                memory: Some(4096),
            },
        )
        .unwrap();
        let xml = domain.render().unwrap();
        assert_eq!(domain.video_model, "virtio");
        assert!(domain.video_vram.is_none());
        assert!(xml.contains("<model type='virtio' heads='1'/>"));
        assert!(xml.contains("OVMF_CODE_4M.fd"));
        assert!(xml.find("<boot dev='cdrom'/>").unwrap() < xml.find("<boot dev='hd'/>").unwrap());
        assert_eq!(config.ztp(), data::Ztp::CloudInit);
        assert!(config.vm().unwrap().cdrom.image.is_none());
    }

    /// The owner's Ed25519 key in every generator test.
    const OWNER_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh";

    /// The rule in an Ignition sudoers file's base64 data URL.
    pub(crate) fn ignition_file_text(file: &serde_json::Value) -> String {
        util::base64_decode(
            file["contents"]["source"]
                .as_str()
                .unwrap()
                .strip_prefix("data:;base64,")
                .unwrap(),
        )
        .unwrap()
    }

    /// The owner's second key on the Flatcar node.
    pub(crate) const FLATCAR_SECOND_KEY: &str = "ssh-rsa AAAAB3NzaC1yc2E second-owner-key";

    /// The Ignition config generated for a Flatcar node whose owner has two keys, as JSON and as
    /// text.
    pub(crate) fn flatcar_ignition_config() -> (serde_json::Value, String) {
        let dir = tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let blank_dir = dir.path().join(SHERPA_BLANK_DISK_DIR);
        fs::create_dir(&blank_dir).unwrap();
        // Generation only copies this fixture; no filesystem is mounted or VM started.
        fs::write(
            blank_dir.join(SHERPA_BLANK_DISK_EXT4_500MB),
            b"disk-fixture",
        )
        .unwrap();
        let node = topology::NodeExpanded {
            name: "vm".to_owned(),
            model_kind: ModelKind::default_for(data::NodeModel::FlatcarLinux),
            ..Default::default()
        };
        let image = data::ModelKind::default_for(data::NodeModel::FlatcarLinux).node_config();
        let account = test_account("alice", &[OWNER_KEY, FLATCAR_SECOND_KEY]);
        let network = nocloud_test_network();
        let (tx, _rx) = mpsc::unbounded_channel();
        let progress = ProgressSender::new(tx);
        let dns = util::default_dns(&network);
        let ztp = NodeZtp {
            lab_dir: root,
            boot_dir: root,
            tftp_dir: root,
            images_dir: root,
            device_password: &account.device_password,
            dns: &dns,
            mgmt_net: &network,
            progress: &progress,
            node_ipv4_address: "192.0.2.10".parse().unwrap(),
            mac_address: "52:54:00:00:00:01",
            node_name_with_lab: "vm-test",
            cert_paths: None,
            guest_account: Some(&account),
        };
        let (config_path, _, _, _) = generate_ignition_ztp(&node, &image, &ztp).unwrap();
        let config = fs::read_to_string(config_path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&config).unwrap();

        (json, config)
    }

    /// The cloud-init user data generated for the owner on each of a few models, as text and as
    /// parsed JSON, with the model it is for.
    pub(crate) fn owner_cloud_init_configs() -> Vec<(data::NodeModel, String, serde_json::Value)> {
        let dir = tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let account = test_account("alice", &[OWNER_KEY]);
        let network = nocloud_test_network();
        let (tx, _rx) = mpsc::unbounded_channel();
        let progress = ProgressSender::new(tx);
        let dns = util::default_dns(&network);
        let ztp = NodeZtp {
            lab_dir: root,
            boot_dir: root,
            tftp_dir: root,
            images_dir: root,
            device_password: &account.device_password,
            dns: &dns,
            mgmt_net: &network,
            progress: &progress,
            node_ipv4_address: "192.0.2.10".parse().unwrap(),
            mac_address: "52:54:00:00:00:01",
            node_name_with_lab: "vm-test",
            cert_paths: None,
            guest_account: Some(&account),
        };
        let mut configs = Vec::new();
        for model in [
            data::NodeModel::UbuntuLinux,
            data::NodeModel::AlpineLinux,
            data::NodeModel::FreeBsd,
            data::NodeModel::WindowsServer,
        ] {
            let node = topology::NodeExpanded {
                name: model.to_string(),
                model_kind: ModelKind::default_for(model),
                ..Default::default()
            };
            let image = data::ModelKind::default_for(model).node_config();
            let node_dir = format!("{root}/{model}");
            match model {
                data::NodeModel::WindowsServer => {
                    windows_cloudbase_init(&node, &ztp, &node_dir).unwrap();
                }
                data::NodeModel::AlpineLinux => alpine_cloud_init(&node, &ztp, &node_dir).unwrap(),
                _ => linux_cloud_init(&node, &image, &ztp, &node_dir).unwrap(),
            }
            let user_data =
                fs::read_to_string(format!("{node_dir}/{CLOUD_INIT_USER_DATA}")).unwrap();
            let parsed: serde_json::Value =
                serde_json::from_str(user_data.strip_prefix("#cloud-config\n").unwrap()).unwrap();
            configs.push((model, user_data, parsed));
        }
        configs
    }

    #[test]
    fn owner_login_024_windows_disables_every_account_but_the_owner_before_its_startup_scripts() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let account = test_account("alice", &[OWNER_KEY]);
        let network = nocloud_test_network();
        let (tx, _rx) = mpsc::unbounded_channel();
        let progress = ProgressSender::new(tx);
        let dns = util::default_dns(&network);
        let ztp = NodeZtp {
            lab_dir: root,
            boot_dir: root,
            tftp_dir: root,
            images_dir: root,
            device_password: &account.device_password,
            dns: &dns,
            mgmt_net: &network,
            progress: &progress,
            node_ipv4_address: "192.0.2.10".parse().unwrap(),
            mac_address: "52:54:00:00:00:01",
            node_name_with_lab: "vm-test",
            cert_paths: None,
            guest_account: Some(&account),
        };
        for model in [
            data::NodeModel::WindowsServer,
            data::NodeModel::DevboxWindows,
        ] {
            let node = topology::NodeExpanded {
                name: model.to_string(),
                model_kind: ModelKind::default_for(model),
                startup_scripts: Some(vec![script("setup.ps1")]),
                ..Default::default()
            };
            let node_dir = format!("{root}/{model}");
            windows_cloudbase_init(&node, &ztp, &node_dir).unwrap();

            let user_data =
                fs::read_to_string(format!("{node_dir}/{CLOUD_INIT_USER_DATA}")).unwrap();
            let parsed: serde_json::Value =
                serde_json::from_str(user_data.strip_prefix("#cloud-config\n").unwrap()).unwrap();
            let users = parsed["users"].as_array().unwrap();
            assert_eq!(users.len(), 1, "{model}");
            assert_eq!(users[0]["name"], "alice", "{model}");
            let script_file = parsed["write_files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|file| file["path"] == WINDOWS_OWNER_ONLY_LOGIN_SCRIPT)
                .unwrap_or_else(|| panic!("{model}: no owner-only login script"));
            let content = script_file["content"].as_str().unwrap();
            assert!(content.contains("Get-LocalUser -Name 'alice'"), "{content}");
            assert!(content.contains("Disable-LocalUser"), "{content}");
            let runcmd: Vec<&str> = parsed["runcmd"]
                .as_array()
                .unwrap()
                .iter()
                .map(|command| command.as_str().unwrap())
                .collect();
            let lockdown = runcmd
                .iter()
                .position(|command| command.contains(WINDOWS_OWNER_ONLY_LOGIN_SCRIPT))
                .unwrap_or_else(|| panic!("{model}: {runcmd:?}"));
            let startup = runcmd
                .iter()
                .position(|command| command.contains(r"startup_scripts\setup.ps1"))
                .unwrap_or_else(|| panic!("{model}: {runcmd:?}"));
            assert!(lockdown < startup, "{model}: {runcmd:?}");
        }
        assert_owner_only_login_script_disables_the_rest();
    }

    /// The rendered Windows script keeps the owner enabled and disables every other account.
    fn assert_owner_only_login_script_disables_the_rest() {
        let output = template::WindowsOwnerOnlyLoginTemplate {
            owner: "alice".to_string(),
        }
        .render()
        .unwrap();
        let commands: Vec<&str> = output
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .collect();
        assert_eq!(
            commands,
            [
                "$ErrorActionPreference = 'Stop'",
                "$owner = Get-LocalUser -Name 'alice'",
                "if (-not $owner.Enabled) {",
                r#"    throw "The lab owner's account 'alice' is disabled, so no other account was disabled""#,
                "}",
                "Get-LocalUser |",
                "    Where-Object { $_.Enabled -and $_.SID -ne $owner.SID } |",
                "    Disable-LocalUser",
            ]
        );
        // Administrator and Admin are among the accounts disabled, never enabled or given a
        // password.
        for account in ["Administrator", "Admin"] {
            assert!(!output.contains(&format!("-Name '{account}'")), "{output}");
        }
        assert!(!output.contains("Enable-LocalUser"));
        assert!(!output.contains("Set-LocalUser"));
    }

    fn nocloud_test_network() -> data::SherpaNetwork {
        data::SherpaNetwork {
            v4: data::NetworkV4 {
                prefix: "192.0.2.0/24".parse().unwrap(),
                first: "192.0.2.1".parse().unwrap(),
                last: "192.0.2.255".parse().unwrap(),
                boot_server: "192.0.2.2".parse().unwrap(),
                network: "192.0.2.0".parse().unwrap(),
                subnet_mask: "255.255.255.0".parse().unwrap(),
                hostmask: "0.0.0.255".parse().unwrap(),
                prefix_length: 24,
            },
            v6: None,
        }
    }

    #[test]
    fn config_delivery_008_nocloud_user_data_is_the_nodes_config_beside_a_static_network_config() {
        let root = tempdir().unwrap();
        let dir = root.path().join("cp1");
        let user_data = "version: v1alpha1\nmachine:\n  type: controlplane\n";
        write_nocloud_files(
            dir.to_str().unwrap(),
            "cp1",
            "192.0.2.10".parse().unwrap(),
            "52:54:00:aa:bb:cc",
            &nocloud_test_network(),
            None,
            Some(user_data),
        )
        .unwrap();

        // The node's own config is delivered byte for byte.
        assert_eq!(
            fs::read_to_string(dir.join(CLOUD_INIT_USER_DATA)).unwrap(),
            user_data
        );
        let meta: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join(CLOUD_INIT_META_DATA)).unwrap())
                .unwrap();
        assert_eq!(meta["local-hostname"], "cp1");
        let network: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join(CLOUD_INIT_NETWORK_CONFIG)).unwrap())
                .unwrap();
        let interface = &network["ethernets"]["id0"];
        assert_eq!(network["version"], 2);
        assert_eq!(interface["match"]["macaddress"], "52:54:00:aa:bb:cc");
        assert_eq!(interface["addresses"][0], "192.0.2.10/24");
        assert_eq!(interface["routes"][0]["via"], "192.0.2.1");
        assert_eq!(interface["nameservers"]["addresses"][0], "192.0.2.2");
    }

    #[test]
    fn config_delivery_009_nocloud_with_no_config_writes_no_user_data() {
        let root = tempdir().unwrap();
        let dir = root.path().join("cp1");
        write_nocloud_files(
            dir.to_str().unwrap(),
            "cp1",
            "192.0.2.10".parse().unwrap(),
            "52:54:00:aa:bb:cc",
            &nocloud_test_network(),
            None,
            None,
        )
        .unwrap();
        assert!(!dir.join(CLOUD_INIT_USER_DATA).exists());
        assert!(dir.join(CLOUD_INIT_META_DATA).exists());
        assert!(dir.join(CLOUD_INIT_NETWORK_CONFIG).exists());
    }

    #[test]
    fn node_ready_018_a_skipped_check_does_not_wait_for_a_containers_ready_port() {
        let check = |port, skip| NodeReadyCheck { port, skip };
        let frr = ModelKind::default_for(data::NodeModel::FrrLinux);
        assert_eq!(container_ready_port(frr, &check(Some(80), None)), Some(80));
        assert_eq!(
            container_ready_port(frr, &check(Some(80), Some(false))),
            Some(80)
        );
        assert_eq!(container_ready_port(frr, &check(None, None)), None);
        assert_eq!(
            container_ready_port(frr, &check(Some(80), Some(true))),
            None
        );
    }

    #[test]
    fn node_ready_006_a_sonic_container_is_checked_on_ssh_unless_the_manifest_says_otherwise() {
        let check = |port, skip| NodeReadyCheck { port, skip };
        let sonic =
            ModelKind::new(data::NodeModel::SonicLinux, Some(data::NodeKind::Container)).unwrap();
        assert_eq!(
            container_ready_port(sonic, &check(None, None)),
            Some(SSH_PORT)
        );
        assert_eq!(
            container_ready_port(sonic, &check(Some(80), None)),
            Some(80)
        );
        assert_eq!(container_ready_port(sonic, &check(None, Some(true))), None);
        // `up` probes the port the node's stored check holds.
        assert_eq!(
            container_ready_port(sonic, &check(None, None)),
            stored_ready_check(sonic, &check(None, None), data::Ztp::Volume).port
        );
    }

    #[test]
    fn node_ready_005_a_node_is_checked_on_its_models_port_unless_the_manifest_sets_one() {
        let check = |port, skip| NodeReadyCheck { port, skip };
        let stored = |port, skip| data::ReadyCheck {
            port,
            skip,
            sshd_marker: false,
        };
        let ubuntu = ModelKind::default_for(data::NodeModel::UbuntuLinux);
        let talos = ModelKind::default_for(data::NodeModel::TalosLinux);
        assert_eq!(
            stored_ready_check(ubuntu, &check(None, None), Ztp::CloudInit),
            stored(Some(SSH_PORT), false)
        );
        assert_eq!(
            stored_ready_check(talos, &check(None, None), Ztp::CloudInit),
            stored(Some(TALOS_API_PORT), false)
        );
        assert_eq!(
            stored_ready_check(ubuntu, &check(Some(8443), Some(true)), Ztp::CloudInit),
            stored(Some(8443), true)
        );
        for model in [
            data::NodeModel::PostgresqlDb,
            data::NodeModel::NanosUnikernel,
        ] {
            let model_kind = ModelKind::default_for(model);
            assert_eq!(
                stored_ready_check(model_kind, &check(None, None), Ztp::CloudInit),
                stored(None, false)
            );
            assert_eq!(
                stored_ready_check(model_kind, &check(Some(80), Some(false)), Ztp::CloudInit),
                stored(Some(80), false)
            );
        }
    }

    #[test]
    fn node_ready_009_a_windows_node_needs_the_marker_only_on_ssh_with_cloud_init() {
        let check = |port| NodeReadyCheck { port, skip: None };
        for model in [
            data::NodeModel::WindowsServer,
            data::NodeModel::DevboxWindows,
        ] {
            let windows = ModelKind::default_for(model);
            assert!(stored_ready_check(windows, &check(None), Ztp::CloudInit).sshd_marker);
            assert!(!stored_ready_check(windows, &check(None), Ztp::Disabled).sshd_marker);
            // Checked on another port, it is checked as that port.
            assert!(!stored_ready_check(windows, &check(Some(3389)), Ztp::CloudInit).sshd_marker);
        }
        let ubuntu = ModelKind::default_for(data::NodeModel::UbuntuLinux);
        assert!(!stored_ready_check(ubuntu, &check(None), Ztp::CloudInit).sshd_marker);
    }

    /// A listener that answers every connection with `reply` and hangs up, such as a web
    /// server or a port forward with nothing behind it.
    async fn listener_replying(reply: &'static [u8]) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let _ = stream.write_all(reply).await;
            }
        });
        (port, task)
    }

    /// An OpenSSH server run as the test's user on a free port of the loopback address, showing
    /// Sherpa's ready marker in its identification line or not, as a Windows node's does once it
    /// runs with Sherpa's config and before. Needs `/usr/sbin/sshd` and `ssh-keygen`.
    pub(crate) struct TestSshd {
        pub(crate) port: u16,
        child: std::process::Child,
        _dir: tempfile::TempDir,
    }

    impl TestSshd {
        pub(crate) fn start(shows_marker: bool) -> Self {
            let dir = tempdir().unwrap();
            let host_key = dir.path().join("host_key");
            let status = Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(&host_key)
                .status()
                .unwrap();
            assert!(status.success());
            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let addendum = if shows_marker {
                SHERPA_SSHD_READY_MARKER
            } else {
                "none"
            };
            let config = format!(
                "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nPidFile none\nUsePAM no\n\
                 VersionAddendum {addendum}\n",
                host_key.display()
            );
            let config_path = dir.path().join("sshd_config");
            fs::write(&config_path, config).unwrap();
            let child = Command::new("/usr/sbin/sshd")
                .args(["-D", "-f"])
                .arg(&config_path)
                .spawn()
                .unwrap();
            let address = SocketAddr::from(([127, 0, 0, 1], port));
            for _ in 0..50 {
                if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Self {
                port,
                child,
                _dir: dir,
            }
        }
    }

    impl Drop for TestSshd {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[tokio::test]
    #[ignore = "runs /usr/sbin/sshd"]
    async fn node_ready_007_a_windows_node_is_not_ready_until_its_ssh_server_shows_the_ready_marker()
     {
        let image_config = TestSshd::start(false);
        assert_eq!(
            check_vm_ready("127.0.0.1", image_config.port, true)
                .await
                .unwrap(),
            VmReadiness::NotReady
        );
        let sherpa_config = TestSshd::start(true);
        assert!(matches!(
            check_vm_ready("127.0.0.1", sherpa_config.port, true)
                .await
                .unwrap(),
            VmReadiness::Ready { host_key: Some(_) }
        ));
    }

    #[tokio::test]
    async fn node_ready_001_on_port_22_a_port_that_is_not_an_ssh_server_is_not_ready() {
        let (port, task) = listener_replying(b"HTTP/1.0 400 Bad Request\r\n\r\n").await;
        assert!(check_node_port_ready("127.0.0.1", port).unwrap());
        assert!(
            check_node_ssh_ready("127.0.0.1", port)
                .await
                .unwrap()
                .is_none()
        );
        task.abort();
    }

    #[tokio::test]
    async fn node_ready_003_a_closed_port_is_not_ready() {
        // Port 1 is almost certainly not listening.
        assert!(!check_node_port_ready("127.0.0.1", 1).unwrap());
        assert!(
            check_node_ssh_ready("127.0.0.1", 1)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn node_ready_002_on_any_other_port_an_accepted_tcp_connection_is_ready() {
        let (port, task) = listener_replying(b"not ssh\r\n").await;
        assert_eq!(
            check_vm_ready("127.0.0.1", port, false).await.unwrap(),
            VmReadiness::Ready { host_key: None }
        );
        task.abort();
    }

    #[tokio::test]
    async fn node_ready_004_an_ssh_server_that_sends_lines_before_its_identification_is_still_ssh()
    {
        let (port, task) =
            listener_replying(b"Welcome\r\nSSH-2.0-OpenSSH_for_Windows_9.5 sherpa-ready\r\n").await;
        assert_eq!(
            ssh_identification("127.0.0.1", port)
                .await
                .unwrap()
                .as_deref(),
            Some("SSH-2.0-OpenSSH_for_Windows_9.5 sherpa-ready")
        );
        task.abort();
    }

    #[test]
    fn node_ready_008_the_ready_marker_counts_only_as_a_whole_identification_comment() {
        let marker = SHERPA_SSHD_READY_MARKER;
        assert!(identification_shows(
            "SSH-2.0-OpenSSH_for_Windows_9.5 sherpa-ready",
            marker
        ));
        assert!(identification_shows(
            "SSH-2.0-OpenSSH_9.6 Ubuntu-3 sherpa-ready",
            marker
        ));
        // The image's own server, before cloudbase-init has given it Sherpa's config.
        assert!(!identification_shows(
            "SSH-2.0-OpenSSH_for_Windows_9.5",
            marker
        ));
        assert!(!identification_shows("SSH-2.0-sherpa-ready", marker));
        assert!(!identification_shows(
            "SSH-2.0-OpenSSH_9.6 not-sherpa-ready",
            marker
        ));
    }

    #[test]
    fn a_windows_node_s_sshd_config_shows_the_ready_marker() {
        let config = template::WindowsSshdConfigTemplate {
            authorized_keys_dir: WINDOWS_SSHD_AUTHORIZED_KEYS_DIR.to_owned(),
            ready_marker: SHERPA_SSHD_READY_MARKER.to_owned(),
        }
        .render()
        .unwrap();
        assert!(
            config.contains(&format!("\nVersionAddendum {SHERPA_SSHD_READY_MARKER}")),
            "{config}"
        );
    }

    /// A lab folder and the accounts and network its nodes are configured with.
    struct ZtpLab {
        dir: tempfile::TempDir,
        account: GuestAccount,
        network: data::SherpaNetwork,
        dns: data::Dns,
        progress: ProgressSender,
        /// Kept open so the status messages the nodes are configured with can be sent.
        _messages: mpsc::UnboundedReceiver<Message>,
    }

    impl ZtpLab {
        fn new() -> Self {
            let network = nocloud_test_network();
            let (tx, messages) = mpsc::unbounded_channel();
            Self {
                dir: tempdir().unwrap(),
                account: test_account(
                    "alice",
                    &[OWNER_KEY, &format!("ssh-rsa {IOL_TEST_RSA_KEY}")],
                ),
                dns: util::default_dns(&network),
                network,
                progress: ProgressSender::new(tx),
                _messages: messages,
            }
        }

        fn root(&self) -> &str {
            self.dir.path().to_str().unwrap()
        }

        fn lab(&self) -> LabZtp<'_> {
            LabZtp {
                lab_id: "a1b2c3d4",
                lab_dir: self.root(),
                images_dir: "/images",
                dns: &self.dns,
                mgmt_net: &self.network,
                progress: &self.progress,
                owner_account: &self.account,
            }
        }

        /// Generate the container node's setup with `ztp` as its manifest setting.
        fn container(
            &self,
            node: &mut topology::NodeExpanded,
            ztp: Option<data::Ztp>,
        ) -> Result<ContainerZtpResult> {
            node.ztp.method = ztp;
            generate_container_ztp(
                node,
                &node.model_kind.node_config(),
                &self.lab(),
                "192.0.2.10".parse().unwrap(),
                None,
            )
        }

        /// The files written under the lab folder, relative to it, sorted.
        fn files(&self) -> Vec<String> {
            fn walk(dir: &Path, root: &Path, found: &mut Vec<String>) {
                for entry in fs::read_dir(dir).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        walk(&path, root, found);
                    } else {
                        found.push(path.strip_prefix(root).unwrap().display().to_string());
                    }
                }
            }
            let mut found = Vec::new();
            walk(self.dir.path(), self.dir.path(), &mut found);
            found.sort();
            found
        }
    }

    fn named_node(name: &str, model: data::NodeModel) -> topology::NodeExpanded {
        topology::NodeExpanded {
            name: name.to_owned(),
            model_kind: ModelKind::default_for(model),
            ..Default::default()
        }
    }

    /// A container node of `model`, which may also take another kind.
    fn named_container(name: &str, model: data::NodeModel) -> topology::NodeExpanded {
        topology::NodeExpanded {
            name: name.to_owned(),
            model_kind: ModelKind::new(model, Some(data::NodeKind::Container)).unwrap(),
            ..Default::default()
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    const MOUNTED_CONFIG_MODELS: [data::NodeModel; 4] = [
        data::NodeModel::AristaEos,
        data::NodeModel::NokiaSrlinux,
        data::NodeModel::JuniperEvolved,
        data::NodeModel::FrrLinux,
    ];

    #[test]
    fn ztp_disabled_005_a_vm_with_ztp_off_is_built_with_its_boot_disk_alone() {
        let lab = ZtpLab::new();
        let mut node = named_node("vm01", data::NodeModel::UbuntuLinux);
        node.ztp.method = Some(data::Ztp::Disabled);
        let image = data::ModelKind::default_for(data::NodeModel::UbuntuLinux).node_config();

        let result = generate_vm_ztp(
            &mut node,
            &image,
            &lab.lab(),
            "192.0.2.10".parse().unwrap(),
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.ztp_record.ztp, data::Ztp::Disabled);
        // The boot disk alone: no CD-ROM, config disk or USB disk.
        assert_eq!(result.clone_disks.len(), 1);
        assert_eq!(result.disks.len(), 1);
        assert_eq!(result.firmware.qemu_commands, Vec::new());
        assert!(result.firmware.ignition_config.is_none());
        assert!(lab.files().is_empty(), "{:?}", lab.files());
    }

    #[test]
    fn config_delivery_014_ignition_is_a_firmware_config_entry_not_a_disk_or_qemu_argument() {
        let lab = ZtpLab::new();
        let images = tempdir().unwrap();
        let blank_disks = images.path().join(SHERPA_BLANK_DISK_DIR);
        fs::create_dir(&blank_disks).unwrap();
        fs::write(blank_disks.join(SHERPA_BLANK_DISK_EXT4_500MB), b"disk").unwrap();
        let lab_ztp = LabZtp {
            images_dir: images.path().to_str().unwrap(),
            ..lab.lab()
        };
        let mut node = named_node("flatcar01", data::NodeModel::FlatcarLinux);
        let image = data::ModelKind::default_for(data::NodeModel::FlatcarLinux).node_config();

        let result = generate_vm_ztp(
            &mut node,
            &image,
            &lab_ztp,
            "192.0.2.10".parse().unwrap(),
            None,
            None,
        )
        .unwrap();

        let pool_config = format!("{SHERPA_STORAGE_POOL_PATH}/flatcar01-a1b2c3d4-cfg.ign");
        assert_eq!(result.ztp_record.ztp, data::Ztp::Ignition);
        assert_eq!(
            result.firmware.ignition_config.as_deref(),
            Some(&*pool_config)
        );
        assert_eq!(result.firmware.qemu_commands, Vec::new());
        // The config is copied into the pool, but is not one of the domain's disks.
        assert!(
            result
                .clone_disks
                .iter()
                .any(|disk| disk.dst == pool_config)
        );
        assert!(result.disks.iter().all(|disk| disk.src_file != pool_config));

        let domain = build_domain_template(
            &node,
            &image,
            "a1b2c3d4",
            "/usr/bin/qemu-system-x86_64",
            DomainParts {
                disks: result.disks,
                interfaces: vec![],
                consoles: ConsoleSockets {
                    serial: "/opt/sherpa/run/console/lab1/1.serial".to_owned(),
                    vnc: "/opt/sherpa/run/console/lab1/1.vnc".to_owned(),
                },
                management_network: "mgmt".to_owned(),
                isolated_network: String::new(),
                reserved_network: String::new(),
            },
            result.firmware,
            &data::NodeResources {
                cpu_count: Some(2),
                memory: Some(2048),
            },
        )
        .unwrap();
        let xml = domain.render().unwrap();
        assert!(
            xml.contains(&format!(
                "<entry name='opt/org.flatcar-linux/config' file='{pool_config}'/>"
            )),
            "{xml}"
        );
        assert!(!xml.contains("-fw_cfg"), "{xml}");
        assert!(!xml.contains("commandline"), "{xml}");
    }

    #[test]
    fn ztp_setting_015_a_vm_with_no_manifest_method_gets_its_images_media() {
        let lab = ZtpLab::new();
        let mut node = named_node("talos01", data::NodeModel::TalosLinux);
        let image = data::ModelKind::default_for(data::NodeModel::TalosLinux).node_config();

        let result = generate_vm_ztp(
            &mut node,
            &image,
            &lab.lab(),
            "192.0.2.10".parse().unwrap(),
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.ztp_record.ztp, data::Ztp::NoCloud);
        assert_eq!(
            result.clone_disks.len(),
            2,
            "the boot disk and the seed ISO"
        );
        assert!(
            lab.files()
                .contains(&format!("{VM_BOOT_DIR}/talos01/{ZTP_ISO}"))
        );
    }

    #[test]
    fn config_delivery_010_cdrom_writes_the_config_under_its_models_file_name_into_an_iso() {
        let lab = ZtpLab::new();
        let mut node = named_node("srx01", data::NodeModel::JuniperSrxv3);
        let image = data::ModelKind::default_for(data::NodeModel::JuniperSrxv3).node_config();

        let result = generate_vm_ztp(
            &mut node,
            &image,
            &lab.lab(),
            "192.0.2.10".parse().unwrap(),
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.ztp_record.ztp, data::Ztp::Cdrom);
        assert_eq!(
            lab.files(),
            [
                format!("{VM_BOOT_DIR}/srx01/{JUNIPER_ZTP_CONFIG}"),
                format!("{VM_BOOT_DIR}/srx01/{ZTP_ISO}"),
            ]
        );
    }

    /// An ISO 9660 sector.
    const ISO_SECTOR: usize = 2048;

    /// The volume label of an ISO image, from its primary volume descriptor.
    fn iso_volume_label(image: &[u8]) -> &str {
        let descriptor = &image[16 * ISO_SECTOR..17 * ISO_SECTOR];
        std::str::from_utf8(&descriptor[40..72]).unwrap().trim_end()
    }

    /// The contents of the file at `path` in an ISO image's primary tree, `None` when there is
    /// no file there. Each folder of `path` is a name, and the file its name with `;1`.
    fn iso_file<'a>(image: &'a [u8], path: &[&str]) -> Option<&'a [u8]> {
        let le32 = |bytes: &[u8]| u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        let root = &image[16 * ISO_SECTOR + 156..16 * ISO_SECTOR + 190];
        let (mut extent, mut size) = (le32(&root[2..]), le32(&root[10..]));
        for name in path {
            let records = &image[extent * ISO_SECTOR..extent * ISO_SECTOR + size];
            let mut at = 0;
            let mut found = None;
            while at < records.len() {
                let len = records[at] as usize;
                if len == 0 {
                    at = (at / ISO_SECTOR + 1) * ISO_SECTOR;
                    continue;
                }
                let record = &records[at..at + len];
                let id_len = record[32] as usize;
                if &record[33..33 + id_len] == name.as_bytes() {
                    found = Some((le32(&record[2..]), le32(&record[10..])));
                    break;
                }
                at += len;
            }
            (extent, size) = found?;
        }
        Some(&image[extent * ISO_SECTOR..extent * ISO_SECTOR + size])
    }

    #[test]
    fn custom_config_011_a_panos_config_is_init_cfg_in_the_config_folder_of_the_bootstrap_iso() {
        let lab = ZtpLab::new();
        let mut node = named_node("panos01", data::NodeModel::PaloaltoPanos);
        let custom = "type=static\nhostname=custom\n";
        node.ztp.config = Some(custom.to_owned());
        let image = data::ModelKind::default_for(data::NodeModel::PaloaltoPanos).node_config();

        let result = generate_vm_ztp(
            &mut node,
            &image,
            &lab.lab(),
            "192.0.2.10".parse().unwrap(),
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.ztp_record.ztp, data::Ztp::Cdrom);
        let iso = fs::read(format!("{}/{VM_BOOT_DIR}/panos01/{ZTP_ISO}", lab.root())).unwrap();
        assert_eq!(iso_volume_label(&iso), "bootstrap");
        assert_eq!(
            iso_file(&iso, &["config", "init-cfg.txt;1"]),
            Some(custom.as_bytes())
        );
        assert_eq!(iso_file(&iso, &["init-cfg.txt;1"]), None, "not at the root");
        assert_eq!(
            iso_file(&iso, &["config", "bootstrap.xml;1"]),
            None,
            "no generated config beside it"
        );
        for folder in ["content", "license", "software"] {
            assert!(iso_file(&iso, &[folder]).is_some(), "{folder} folder");
        }
    }

    #[test]
    fn device_accounts_022_srx_owner_and_root_have_a_hash_of_the_password() {
        let lab = ZtpLab::new();
        let mut node = named_node("srx01", data::NodeModel::JuniperSrxv3);
        let image = data::ModelKind::default_for(data::NodeModel::JuniperSrxv3).node_config();

        generate_vm_ztp(
            &mut node,
            &image,
            &lab.lab(),
            "192.0.2.10".parse().unwrap(),
            None,
            None,
        )
        .unwrap();

        let config = fs::read_to_string(format!(
            "{}/{VM_BOOT_DIR}/srx01/{JUNIPER_ZTP_CONFIG}",
            lab.root()
        ))
        .unwrap();
        assert!(config.contains("user alice {"), "{config}");
        assert_eq!(
            config.matches(r#"encrypted-password "$6$"#).count(),
            2,
            "the owner and root: {config}"
        );
        assert!(config.contains(r#"ssh-ed25519 ""#), "{config}");
    }

    #[test]
    fn config_delivery_024_volume_on_a_vm_fails_as_not_validated() {
        let lab = ZtpLab::new();
        let mut node = named_node("vm01", data::NodeModel::UbuntuLinux);
        node.ztp.method = Some(data::Ztp::Volume);
        let image = data::ModelKind::default_for(data::NodeModel::UbuntuLinux).node_config();

        let error = generate_vm_ztp(
            &mut node,
            &image,
            &lab.lab(),
            "192.0.2.10".parse().unwrap(),
            None,
            None,
        )
        .err()
        .expect("the setting is refused")
        .to_string();

        assert!(error.contains("'volume'"), "{error}");
        assert!(error.contains("was not validated"), "{error}");
        assert!(lab.files().is_empty(), "{:?}", lab.files());
    }

    #[test]
    fn config_delivery_018_ceos_volume_mounts_its_config_and_copies_it_into_flash() {
        let lab = ZtpLab::new();
        let mut node = named_container("ceos01", data::NodeModel::AristaEos);

        let result = lab.container(&mut node, None).unwrap();

        let config = format!("{}/ceos01/ceos01.conf", lab.root());
        assert_eq!(result.ztp_record.ztp, data::Ztp::Volume);
        assert_eq!(lab.files(), ["ceos01/ceos01.conf"]);
        assert_eq!(
            result.volumes,
            [format!("{config}:{ARISTA_CEOS_ZTP_VOLUME_MOUNT}:ro")]
        );
        assert_eq!(result.commands, ceos_startup_commands());
        let with_ztp = result;

        let lab = ZtpLab::new();
        let mut node = named_container("ceos01", data::NodeModel::AristaEos);

        let result = lab.container(&mut node, Some(data::Ztp::Disabled)).unwrap();

        assert_eq!(result.ztp_record.ztp, data::Ztp::Disabled);
        assert!(lab.files().is_empty(), "{:?}", lab.files());
        assert!(result.volumes.is_empty(), "{:?}", result.volumes);
        assert_eq!(result.commands, strings(CONTAINER_ARISTA_CEOS_COMMANDS));
        assert_eq!(result.image, with_ztp.image);
        assert_eq!(result.env_vars, with_ztp.env_vars);
        assert!(result.privileged);
    }

    #[test]
    fn config_delivery_019_srlinux_volume_mounts_its_config_only_with_ztp_on() {
        let lab = ZtpLab::new();
        let mut node = named_node("srl01", data::NodeModel::NokiaSrlinux);
        let with_ztp = lab.container(&mut node, None).unwrap();
        let config = format!("{}/srl01/srl01.json", lab.root());
        assert_eq!(lab.files(), ["srl01/srl01.json"]);
        assert_eq!(
            with_ztp.volumes,
            [format!("{config}:{NOKIA_SRLINUX_ZTP_VOLUME_MOUNT}")]
        );

        let lab = ZtpLab::new();
        let mut node = named_node("srl01", data::NodeModel::NokiaSrlinux);
        let off = lab.container(&mut node, Some(data::Ztp::Disabled)).unwrap();
        assert!(lab.files().is_empty(), "{:?}", lab.files());
        assert!(off.volumes.is_empty(), "{:?}", off.volumes);
        // The start settings are the same either way.
        assert_eq!(off.image, with_ztp.image);
        assert_eq!(off.env_vars, with_ztp.env_vars);
        assert_eq!(off.commands, with_ztp.commands);
        assert_eq!(off.commands, strings(CONTAINER_NOKIA_SRLINUX_COMMANDS));
        assert_eq!(off.user, Some(CONTAINER_NOKIA_SRLINUX_USER.to_owned()));
        assert_eq!(off.user, with_ztp.user);
        assert!(off.privileged);
    }

    #[test]
    fn config_delivery_020_cevolved_volume_mounts_its_config_directory_only_with_ztp_on() {
        let lab = ZtpLab::new();
        let mut node = named_container("ptx01", data::NodeModel::JuniperEvolved);
        let with_ztp = lab.container(&mut node, None).unwrap();
        assert_eq!(
            lab.files(),
            [format!("ptx01/configdisk/{JUNIPER_ZTP_CONFIG}")]
        );
        assert_eq!(
            with_ztp.volumes,
            [format!(
                "{}/ptx01/configdisk:{JUNIPER_CEVOLVED_CONFIG_DIR}:ro",
                lab.root()
            )]
        );

        let lab = ZtpLab::new();
        let mut node = named_container("ptx01", data::NodeModel::JuniperEvolved);
        let off = lab.container(&mut node, Some(data::Ztp::Disabled)).unwrap();
        assert!(lab.files().is_empty(), "{:?}", lab.files());
        assert!(off.volumes.is_empty(), "{:?}", off.volumes);
        assert_eq!(off.image, with_ztp.image);
        assert!(off.privileged);
    }

    #[test]
    fn config_delivery_021_frr_volume_mounts_its_files_and_runs_its_script_only_with_ztp_on() {
        let lab = ZtpLab::new();
        let mut node = named_node("frr01", data::NodeModel::FrrLinux);
        let with_ztp = lab.container(&mut node, None).unwrap();
        let dir = format!("{}/frr01", lab.root());
        assert_eq!(
            lab.files(),
            ["frr01/daemons", "frr01/frr.conf", "frr01/sherpa-init.sh"]
        );
        assert_eq!(
            with_ztp.volumes,
            [
                format!("{dir}/daemons:{FRR_ZTP_DAEMONS_MOUNT}"),
                format!("{dir}/frr.conf:{FRR_ZTP_CONFIG_MOUNT}"),
                format!("{dir}/sherpa-init.sh:{FRR_ZTP_STARTUP_MOUNT}"),
            ]
        );
        assert_eq!(with_ztp.commands, ["/bin/sh", FRR_ZTP_STARTUP_MOUNT]);

        let lab = ZtpLab::new();
        let mut node = named_node("frr01", data::NodeModel::FrrLinux);
        let off = lab.container(&mut node, Some(data::Ztp::Disabled)).unwrap();
        assert!(lab.files().is_empty(), "{:?}", lab.files());
        assert!(off.volumes.is_empty(), "{:?}", off.volumes);
        // The image's own entrypoint runs it.
        assert!(off.commands.is_empty(), "{:?}", off.commands);
        assert_eq!(off.image, with_ztp.image);
        assert_eq!(off.env_vars, with_ztp.env_vars);
        assert!(off.privileged);
    }

    #[test]
    fn config_delivery_023_sonic_container_volume_mounts_its_start_files_and_config_read_only() {
        let lab = ZtpLab::new();
        let mut node = named_container("sonic01", data::NodeModel::SonicLinux);
        let result = lab.container(&mut node, None).unwrap();

        let dir = format!("{}/sonic01", lab.root());
        assert_eq!(result.ztp_record.ztp, data::Ztp::Volume);
        assert_eq!(
            lab.files(),
            [
                "sonic01/config_db.json",
                "sonic01/sherpa-init.sh",
                "sonic01/supervisord.conf"
            ]
        );
        assert_eq!(
            result.volumes,
            [
                format!("{dir}/supervisord.conf:{SONIC_CONTAINER_SUPERVISORD_MOUNT}:ro"),
                format!("{dir}/sherpa-init.sh:{SONIC_CONTAINER_INIT_MOUNT}:ro"),
                format!("{dir}/config_db.json:{SONIC_CONTAINER_CONFIG_DB_MOUNT}:ro"),
            ]
        );
        assert_eq!(result.commands, ["-c", "/mnt/sherpa-ztp/supervisord.conf"]);
        assert!(result.privileged);
        assert_eq!(result.image, "");

        let mounts = node.container.unwrap().volumes.unwrap();
        assert_eq!(mounts.len(), 3);
    }

    #[test]
    fn ztp_disabled_007_a_sonic_container_with_ztp_off_still_gets_its_start_script() {
        let lab = ZtpLab::new();
        let mut node = named_container("sonic01", data::NodeModel::SonicLinux);
        node.interfaces.data_count = Some(4);
        let off = lab.container(&mut node, Some(data::Ztp::Disabled)).unwrap();

        let dir = format!("{}/sonic01", lab.root());
        assert_eq!(
            lab.files(),
            ["sonic01/sherpa-init.sh", "sonic01/supervisord.conf"]
        );
        assert_eq!(
            off.volumes,
            [
                format!("{dir}/supervisord.conf:{SONIC_CONTAINER_SUPERVISORD_MOUNT}:ro"),
                format!("{dir}/sherpa-init.sh:{SONIC_CONTAINER_INIT_MOUNT}:ro"),
            ]
        );
        assert_eq!(off.commands, strings(CONTAINER_SONIC_COMMANDS));
        assert!(off.privileged);
        let init = fs::read_to_string(format!("{dir}/sherpa-init.sh")).unwrap();
        assert!(init.contains("DATA_COUNT=4\n"));
        assert!(!init.contains("useradd"));
        assert!(!init.contains("sonic-cfggen -j"));
    }

    /// A `SONiC` container node with a custom `config_db.json` mounts it and still adds the owner.
    pub(crate) fn assert_sonic_container_with_a_custom_config_adds_the_owner() {
        let lab = ZtpLab::new();
        let mut node = named_container("sonic01", data::NodeModel::SonicLinux);
        node.ztp.config = Some(r#"{"DEVICE_METADATA": {}}"#.to_owned());
        lab.container(&mut node, None).unwrap();

        let dir = format!("{}/sonic01", lab.root());
        assert_eq!(
            fs::read_to_string(format!("{dir}/config_db.json")).unwrap(),
            r#"{"DEVICE_METADATA": {}}"#
        );
        let init = fs::read_to_string(format!("{dir}/sherpa-init.sh")).unwrap();
        assert!(init.contains("useradd -m -s /bin/bash -G frrvty alice"));
    }

    const IOL_MODELS: [data::NodeModel; 2] =
        [data::NodeModel::CiscoIol, data::NodeModel::CiscoIoll2];

    /// An RSA public key, which IOS takes as an MD5 `key-hash`.
    const IOL_TEST_RSA_KEY: &str = "AAAAB3NzaC1yc2EAAAADAQABAAAAgQDGEYEVRGlFrL++QeLlNvvFXeyzaAz9rlYEQIgvCRNqCDvYz/6dHMa8PSnaDTq45TBaIpFvaq7VExU80ny8zLzg94fVrTS3PvJkYvc8cMo5Dr7jX3x6OUGbCSDLKjFklhN4CUGFxB7oSB/5u2XdLViODw3XE2mjg0IcrkFGyFWlWw==";

    /// An IOL node, the `index`th node of its lab.
    fn iol_node(name: &str, model: data::NodeModel, index: u16) -> topology::NodeExpanded {
        topology::NodeExpanded {
            index,
            ..named_node(name, model)
        }
    }

    #[test]
    fn ssh_access_009_an_owner_with_an_rsa_key_of_4096_bits_or_less_logs_in_to_iol() {
        for model in IOL_MODELS {
            let mut lab = ZtpLab::new();
            lab.account = test_account("alice", &[OWNER_KEY, RSA_8192_KEY, RSA_4096_KEY]);
            let mut node = iol_node("r1", model, 1);

            lab.container(&mut node, None).unwrap();

            let config = fs::read_to_string(format!(
                "{}/r1/config/{CISCO_IOL_STARTUP_CONFIG}",
                lab.root()
            ))
            .unwrap();
            assert!(
                config.contains(
                    "ip ssh pubkey-chain\n  username alice\n   key-hash ssh-rsa D9B93808AFAEBEB58DDFAB3225686F61\n!"
                ),
                "{model}: {config}"
            );
        }
    }

    #[test]
    fn config_delivery_022_iol_volume_mounts_its_config_directory_beside_its_nvram() {
        for model in IOL_MODELS {
            let lab = ZtpLab::new();
            let mut node = iol_node("r1", model, 3);

            let result = lab.container(&mut node, None).unwrap();

            let dir = format!("{}/r1", lab.root());
            assert_eq!(result.ztp_record.ztp, data::Ztp::Volume, "{model}");
            assert_eq!(
                lab.files(),
                [format!("r1/config/{CISCO_IOL_STARTUP_CONFIG}")],
                "{model}"
            );
            assert_eq!(
                result.volumes,
                [
                    format!("{dir}/run:{CISCO_IOL_RUN_DIR}"),
                    format!("{dir}/config:{CISCO_IOL_CONFIG_DIR}:ro"),
                ],
                "{model}"
            );
            assert!(Path::new(&format!("{dir}/run")).is_dir(), "{model}");
        }
    }

    #[test]
    fn iol_runs_unprivileged_with_net_admin_and_its_launcher_settings() {
        for model in IOL_MODELS {
            let lab = ZtpLab::new();
            let mut node = iol_node("r1", model, 3);

            let result = lab.container(&mut node, None).unwrap();

            assert!(!result.privileged, "{model}");
            assert_eq!(result.capabilities, ["NET_ADMIN"], "{model}");
            assert_eq!(
                result.env_vars,
                ["IOL_ID=3", "IOL_MEMORY_MB=768", "IOL_ETH_PORTS=16"],
                "{model}"
            );
            assert!(result.commands.is_empty(), "{model}: {:?}", result.commands);
            assert_eq!(node.environment_variables, Some(result.env_vars), "{model}");
        }
    }

    #[test]
    fn iol_takes_the_manifest_memory_and_data_interface_count() {
        let lab = ZtpLab::new();
        let mut node = iol_node("r1", data::NodeModel::CiscoIol, 1);
        node.memory = Some(1024);
        node.interfaces.data_count = Some(5);

        let result = lab.container(&mut node, None).unwrap();

        assert_eq!(
            result.env_vars,
            ["IOL_ID=1", "IOL_MEMORY_MB=1024", "IOL_ETH_PORTS=6"]
        );
    }

    #[test]
    fn ztp_disabled_008_iol_with_ztp_off_has_no_config_mount_and_keeps_its_nvram_and_mounts() {
        let lab = ZtpLab::new();
        let mut node = iol_node("r1", data::NodeModel::CiscoIol, 2);
        node.container.get_or_insert_default().volumes = Some(manifest_volume());

        let result = lab.container(&mut node, Some(data::Ztp::Disabled)).unwrap();

        assert_eq!(result.ztp_record.ztp, data::Ztp::Disabled);
        assert!(lab.files().is_empty(), "{:?}", lab.files());
        assert_eq!(
            result.volumes,
            [
                format!("{}/r1/run:{CISCO_IOL_RUN_DIR}", lab.root()),
                "/srv/own.conf:/etc/own.conf".to_owned(),
            ]
        );
        assert_eq!(
            result.env_vars,
            ["IOL_ID=2", "IOL_MEMORY_MB=768", "IOL_ETH_PORTS=16"]
        );
    }

    #[test]
    fn custom_config_008_an_iol_config_is_written_as_its_startup_config() {
        let lab = ZtpLab::new();
        let mut node = iol_node("r1", data::NodeModel::CiscoIol, 1);
        node.ztp.config = Some("hostname custom\nend\n".to_owned());

        lab.container(&mut node, None).unwrap();

        assert_eq!(
            fs::read_to_string(format!(
                "{}/r1/config/{CISCO_IOL_STARTUP_CONFIG}",
                lab.root()
            ))
            .unwrap(),
            "hostname custom\nend\n"
        );
    }

    #[test]
    fn iol_application_ids_are_the_node_index_from_1_to_999() {
        assert_eq!(iol_app_id("r1", 1).unwrap(), 1);
        assert_eq!(iol_app_id("r1", 999).unwrap(), 999);
        for index in [0, 1000, u16::MAX] {
            let error = iol_app_id("r1", index).unwrap_err().to_string();
            assert!(error.contains("r1"), "{error}");
            assert!(error.contains("999"), "{error}");
        }
    }

    #[test]
    fn an_iol_node_past_the_999th_is_refused() {
        let lab = ZtpLab::new();
        let mut node = iol_node("r1000", data::NodeModel::CiscoIol, 1000);
        let error = lab.container(&mut node, None).err().unwrap().to_string();
        assert!(error.contains("r1000"), "{error}");
    }

    #[test]
    fn config_delivery_025_a_method_other_than_volume_on_a_container_fails_as_not_validated() {
        for model in MOUNTED_CONFIG_MODELS {
            for ztp in [
                data::Ztp::CloudInit,
                data::Ztp::Cdrom,
                data::Ztp::Disk,
                data::Ztp::Http,
                data::Ztp::Ignition,
                data::Ztp::NoCloud,
                data::Ztp::Tftp,
                data::Ztp::Usb,
            ] {
                let lab = ZtpLab::new();
                let mut node = named_container("node1", model);
                let error = lab
                    .container(&mut node, Some(ztp))
                    .err()
                    .expect("the setting is refused")
                    .to_string();
                assert!(
                    error.contains("was not validated"),
                    "{model} {ztp}: {error}"
                );
                assert!(lab.files().is_empty(), "{model} {ztp}");
            }
        }
    }

    #[test]
    fn config_delivery_026_volume_on_a_model_without_a_generator_fails_naming_it() {
        let lab = ZtpLab::new();
        let mut node = named_node("db01", data::NodeModel::MongoDb);
        let error = lab
            .container(&mut node, Some(data::Ztp::Volume))
            .err()
            .expect("the setting is refused")
            .to_string();
        assert!(error.contains("was not validated"), "{error}");
        assert!(error.contains("mongo_db"), "{error}");
    }

    #[test]
    fn container_run_001_a_container_of_a_model_without_start_settings_keeps_the_manifests() {
        for model in [
            data::NodeModel::MysqlDb,
            data::NodeModel::PostgresqlDb,
            data::NodeModel::Generic,
        ] {
            let lab = ZtpLab::new();
            let mut node = named_container("app01", model);
            node.image = Some("example/app".to_owned());
            node.container.get_or_insert_default().commands = Some(strings(&["serve"]));
            node.environment_variables = Some(strings(&["KEY=value"]));
            let result = lab.container(&mut node, None).unwrap();
            assert_eq!(result.ztp_record.ztp, data::Ztp::Disabled, "{model}");
            assert_eq!(result.image, "example/app", "{model}");
            assert_eq!(result.commands, ["serve"], "{model}");
            assert_eq!(result.env_vars, ["KEY=value"], "{model}");
            assert!(lab.files().is_empty(), "{model}");
        }
    }

    #[test]
    fn custom_config_007_vaults_config_is_mounted_as_it_is_with_ztp_off() {
        let lab = ZtpLab::new();
        let mut node = named_node("vault01", data::NodeModel::HashicorpVault);
        node.ztp.config = Some("ui = true".to_owned());

        let result = lab.container(&mut node, None).unwrap();

        let config = format!("{}/vault01/config.hcl", lab.root());
        assert_eq!(result.ztp_record.ztp, data::Ztp::Disabled);
        assert_eq!(
            result.volumes,
            [format!("{config}:{VAULT_ZTP_CONFIG_MOUNT}")]
        );
        assert_eq!(result.commands, strings(CONTAINER_HASHICORP_VAULT_COMMANDS));
        assert_eq!(fs::read_to_string(config).unwrap(), "ui = true");
    }

    #[test]
    fn keycloak_keeps_its_start_settings() {
        let lab = ZtpLab::new();
        let mut node = named_node("kc01", data::NodeModel::KeycloakServer);
        let certs = NodeCertPaths {
            ca_cert: "/certs/ca.crt".to_owned(),
            node_cert: "/certs/node.crt".to_owned(),
            node_key: "/certs/node.key".to_owned(),
        };

        let result = generate_container_ztp(
            &mut node,
            &data::ModelKind::default_for(data::NodeModel::KeycloakServer).node_config(),
            &lab.lab(),
            "192.0.2.10".parse().unwrap(),
            Some(&certs),
        )
        .unwrap();

        assert_eq!(result.ztp_record.ztp, data::Ztp::Disabled);
        assert_eq!(result.commands, strings(CONTAINER_KEYCLOAK_COMMANDS));
        assert_eq!(result.user, Some(CONTAINER_KEYCLOAK_USER.to_owned()));
        assert_eq!(result.volumes.len(), 3, "the certificate mounts");
        assert_eq!(lab.files(), ["keycloak/kc01.toml"]);
    }

    /// The value of `key` in a container's `KEY=value` environment.
    fn env_value<'a>(env_vars: &'a [String], key: &str) -> Option<&'a str> {
        env_vars
            .iter()
            .find_map(|var| var.strip_prefix(key)?.strip_prefix('='))
    }

    /// A Keycloak node's container settings, generated in `lab` as `up` and `redeploy` do.
    fn keycloak_settings(lab: &ZtpLab, name: &str) -> ContainerZtpResult {
        let mut node = named_node(name, data::NodeModel::KeycloakServer);
        let certs = NodeCertPaths {
            ca_cert: "/certs/ca.crt".to_owned(),
            node_cert: "/certs/node.crt".to_owned(),
            node_key: "/certs/node.key".to_owned(),
        };
        generate_container_ztp(
            &mut node,
            &data::ModelKind::default_for(data::NodeModel::KeycloakServer).node_config(),
            &lab.lab(),
            "192.0.2.10".parse().unwrap(),
            Some(&certs),
        )
        .unwrap()
    }

    #[test]
    fn lab_credentials_014_a_keycloak_container_starts_with_its_saved_admin_kept_across_redeploy() {
        let lab = ZtpLab::new();
        let kc01 = keycloak_settings(&lab, "kc01");
        assert_eq!(
            env_value(&kc01.env_vars, "KC_BOOTSTRAP_ADMIN_USERNAME"),
            Some("admin")
        );
        let password = env_value(&kc01.env_vars, "KC_BOOTSTRAP_ADMIN_PASSWORD")
            .unwrap()
            .to_owned();
        assert_eq!(password.len(), 20);

        // Saved for the server's realm setup and for `download`.
        let saved = format!("{}/{KEYCLOAK_DIR}/kc01.toml", lab.root());
        assert!(fs::read_to_string(&saved).unwrap().contains(&password));

        // A second node gets its own; a redeploy keeps the node's.
        let kc02 = keycloak_settings(&lab, "kc02");
        assert_ne!(
            env_value(&kc02.env_vars, "KC_BOOTSTRAP_ADMIN_PASSWORD"),
            Some(password.as_str())
        );
        let redeployed = keycloak_settings(&lab, "kc01");
        assert_eq!(
            env_value(&redeployed.env_vars, "KC_BOOTSTRAP_ADMIN_PASSWORD"),
            Some(password.as_str())
        );
    }

    #[test]
    fn lab_credentials_016_a_generated_password_is_in_no_log_line() {
        let lab = ZtpLab::new();
        let ((keycloak, surreal_db), output) = capture_log(|| {
            let mut db01 = named_node("db01", data::NodeModel::SurrealDb);
            // Created, then redeployed.
            keycloak_settings(&lab, "kc01");
            lab.container(&mut db01, None).unwrap();
            let mut db01 = named_node("db01", data::NodeModel::SurrealDb);
            (
                keycloak_settings(&lab, "kc01"),
                lab.container(&mut db01, None).unwrap(),
            )
        });

        assert!(output.contains("db01"), "the setup was logged: {output}");
        for password in [
            env_value(&keycloak.env_vars, "KC_BOOTSTRAP_ADMIN_PASSWORD").unwrap(),
            env_value(&surreal_db.env_vars, "SURREAL_PASS").unwrap(),
        ] {
            assert!(!output.contains(password), "{output}");
        }
    }

    #[test]
    fn lab_credentials_015_a_surreal_db_container_starts_with_its_saved_root_kept_across_redeploy()
    {
        let lab = ZtpLab::new();
        let mut db01 = named_node("db01", data::NodeModel::SurrealDb);
        let created = lab.container(&mut db01, None).unwrap();
        assert_eq!(env_value(&created.env_vars, "SURREAL_USER"), Some("root"));
        let password = env_value(&created.env_vars, "SURREAL_PASS")
            .unwrap()
            .to_owned();
        assert_eq!(password.len(), 20);
        // Not on the command line, which every user of the host can read.
        assert!(created.commands.iter().all(|arg| !arg.contains(&password)));

        let saved = format!("{}/{SURREAL_DB_DIR}/db01.toml", lab.root());
        assert!(fs::read_to_string(&saved).unwrap().contains(&password));

        let mut db02 = named_node("db02", data::NodeModel::SurrealDb);
        let other = lab.container(&mut db02, None).unwrap();
        assert_ne!(
            env_value(&other.env_vars, "SURREAL_PASS"),
            Some(password.as_str())
        );
        let mut db01 = named_node("db01", data::NodeModel::SurrealDb);
        let redeployed = lab.container(&mut db01, None).unwrap();
        assert_eq!(
            env_value(&redeployed.env_vars, "SURREAL_PASS"),
            Some(password.as_str())
        );
    }

    #[test]
    fn a_surreal_db_node_logs_at_info_so_no_request_body_reaches_its_logs() {
        // At `trace` SurrealDB logs request bodies, a `/signin` password among them.
        let lab = ZtpLab::new();
        let mut node = named_node("db01", data::NodeModel::SurrealDb);
        let commands = lab.container(&mut node, None).unwrap().commands;
        assert!(
            commands.windows(2).any(|pair| pair == ["--log", "info"]),
            "{commands:?}"
        );
        assert!(
            commands.iter().all(|arg| !arg.contains("trace")),
            "{commands:?}"
        );
    }

    fn manifest_volume() -> Vec<topology::VolumeMount> {
        vec![topology::VolumeMount {
            src: "/srv/own.conf".to_owned(),
            dst: "/etc/own.conf".to_owned(),
        }]
    }

    #[test]
    fn ztp_disabled_009_manifest_volumes_reach_a_mounted_config_model_only_with_ztp_off() {
        for model in MOUNTED_CONFIG_MODELS {
            let lab = ZtpLab::new();
            let mut node = named_container("node1", model);
            node.container.get_or_insert_default().volumes = Some(manifest_volume());
            let off = lab.container(&mut node, Some(data::Ztp::Disabled)).unwrap();
            assert_eq!(off.volumes, ["/srv/own.conf:/etc/own.conf"], "{model}");

            let lab = ZtpLab::new();
            let mut node = named_container("node1", model);
            node.container.get_or_insert_default().volumes = Some(manifest_volume());
            let with_ztp = lab.container(&mut node, None).unwrap();
            assert!(
                !with_ztp
                    .volumes
                    .iter()
                    .any(|volume| volume.contains("own.conf")),
                "{model}: {:?}",
                with_ztp.volumes
            );
            assert!(!with_ztp.volumes.is_empty(), "{model}");
        }
    }

    #[test]
    fn manifest_volumes_are_still_dropped_for_gitlab_surrealdb_and_vault() {
        for model in [
            data::NodeModel::GitlabCe,
            data::NodeModel::SurrealDb,
            data::NodeModel::HashicorpVault,
        ] {
            let lab = ZtpLab::new();
            let mut node = named_node("app01", model);
            node.container.get_or_insert_default().volumes = Some(manifest_volume());
            node.ztp.config = Some("ui = true".to_owned());
            let result = lab.container(&mut node, None).unwrap();
            assert!(
                !result
                    .volumes
                    .iter()
                    .any(|volume| volume.contains("own.conf")),
                "{model}: {:?}",
                result.volumes
            );
        }
        // The other application containers take them, as before.
        let lab = ZtpLab::new();
        let mut node = named_node("db01", data::NodeModel::MongoDb);
        node.container.get_or_insert_default().volumes = Some(manifest_volume());
        let result = lab.container(&mut node, None).unwrap();
        assert_eq!(result.volumes, ["/srv/own.conf:/etc/own.conf"]);
    }

    /// A lab folder with the network boot files of nodes `sw1` and `sw10`: each one's TFTP
    /// config and HTTP files.
    fn network_boot_files() -> tempfile::TempDir {
        let lab = tempdir().unwrap();
        let tftp = lab.path().join(ZTP_DIR).join(TFTP_DIR);
        let configs = lab.path().join(ZTP_DIR).join(NODE_CONFIGS_DIR);
        fs::create_dir_all(&tftp).unwrap();
        fs::create_dir_all(&configs).unwrap();
        for node in ["sw1", "sw10"] {
            fs::write(tftp.join(format!("{node}.conf")), "tftp").unwrap();
            for file in http_ztp_files(node) {
                fs::write(configs.join(file), "http").unwrap();
            }
        }
        lab
    }

    /// The network boot files left in `lab`, relative to its ZTP folder, sorted.
    fn network_boot_files_in(lab: &Path) -> Vec<String> {
        let mut found = Vec::new();
        for dir in [TFTP_DIR, NODE_CONFIGS_DIR] {
            for entry in fs::read_dir(lab.join(ZTP_DIR).join(dir)).unwrap() {
                let name = entry.unwrap().file_name().to_string_lossy().into_owned();
                found.push(format!("{dir}/{name}"));
            }
        }
        found.sort();
        found
    }

    #[test]
    fn boot_server_018_a_method_change_removes_only_that_nodes_old_files() {
        let lab = network_boot_files();
        let lab_dir = lab.path().to_str().unwrap();

        // From TFTP, as it boots by CD-ROM now.
        remove_stale_network_boot_files(lab_dir, "sw1", data::Ztp::Cdrom).unwrap();

        assert_eq!(
            network_boot_files_in(lab.path()),
            [
                "configs/sw10.conf",
                "configs/sw10_config_db.json",
                "configs/sw10_ztp_user.sh",
                "tftp/sw10.conf",
            ]
        );
    }

    #[test]
    fn boot_server_019_the_files_of_the_nodes_method_are_kept() {
        let lab = network_boot_files();
        let lab_dir = lab.path().to_str().unwrap();

        remove_stale_network_boot_files(lab_dir, "sw1", data::Ztp::Tftp).unwrap();
        remove_stale_network_boot_files(lab_dir, "sw10", data::Ztp::Http).unwrap();

        let left = network_boot_files_in(lab.path());
        for kept in [
            "configs/sw10.conf",
            "configs/sw10_config_db.json",
            "configs/sw10_ztp_user.sh",
            "tftp/sw1.conf",
        ] {
            assert!(left.iter().any(|file| file == kept), "{kept}: {left:?}");
        }
    }

    /// VM `vm1`'s boot folder in a temporary lab folder, holding its seed and the ISO made from
    /// it; and the ISO as a disk to copy to `dst`.
    fn vm_boot_files(dst: &str) -> (tempfile::TempDir, PathBuf, data::CloneDisk) {
        let lab = tempdir().unwrap();
        let boot_dir = PathBuf::from(vm_boot_dir(lab.path().to_str().unwrap(), "vm1"));
        fs::create_dir_all(&boot_dir).unwrap();
        fs::write(boot_dir.join(CLOUD_INIT_USER_DATA), "#cloud-config\n").unwrap();
        let iso = boot_dir.join(ZTP_ISO);
        fs::write(&iso, vec![0_u8; 2048]).unwrap();
        let disk = copied_disk(iso.to_str().unwrap(), dst);
        (lab, boot_dir, disk)
    }

    #[tokio::test]
    #[ignore = "requires libvirt with the sherpa storage pool: copies a volume into it"]
    async fn config_privacy_004_a_vms_boot_files_are_removed_once_its_media_are_in_the_pool() {
        let conn = Arc::new(Qemu::default().connect().unwrap());
        let dst = format!(
            "{SHERPA_STORAGE_POOL_PATH}/vm1-boot-test{}.iso",
            std::process::id()
        );
        let (lab, boot_dir, disk) = vm_boot_files(&dst);
        let (tx, _rx) = mpsc::unbounded_channel();

        let result = clone_node_disks(
            Arc::clone(&conn),
            vec![disk],
            "boot-test",
            &ProgressSender::new(tx),
            Vec::new(),
            std::slice::from_ref(&boot_dir),
        )
        .await;
        let in_pool = Path::new(&dst).exists();
        remove_created_disks(Arc::clone(&conn), vec![dst.clone()])
            .await
            .unwrap();

        result.unwrap();
        assert!(in_pool, "{dst} was not created");
        assert!(!boot_dir.exists());
        assert!(lab.path().exists());
    }

    #[tokio::test]
    #[ignore = "requires libvirt: connects to it, though the failed copy never reaches it"]
    async fn config_privacy_005_a_vms_boot_files_are_kept_when_its_media_fail_to_upload() {
        let conn = Arc::new(Qemu::default().connect().unwrap());
        // The destination exists, so the copy is refused.
        let taken = tempdir().unwrap();
        let dst = taken.path().join("vm1.iso");
        fs::write(&dst, "taken").unwrap();
        let (_lab, boot_dir, disk) = vm_boot_files(dst.to_str().unwrap());
        let (tx, _rx) = mpsc::unbounded_channel();

        let result = clone_node_disks(
            conn,
            vec![disk],
            "boot-test",
            &ProgressSender::new(tx),
            Vec::new(),
            std::slice::from_ref(&boot_dir),
        )
        .await;

        assert!(result.is_err());
        assert!(boot_dir.join(CLOUD_INIT_USER_DATA).exists());
        assert!(boot_dir.join(ZTP_ISO).exists());
    }

    /// Marks a test process started by `under_a_permissive_umask`, and holds the umask it runs
    /// under, in octal.
    const UMASK_ENV: &str = "SHERPA_TEST_NODE_CONFIG_UMASK";

    /// Run the test `test` of this module again in a child process whose umask takes nothing
    /// away, so the modes it checks come from Sherpa and not from the umask; the umask is per
    /// process. True in the child, which has set its umask and does the check; false in the
    /// parent, once the child has passed.
    fn under_a_permissive_umask(test: &str) -> bool {
        if let Ok(mask) = env::var(UMASK_ENV) {
            umask(Mode::from_bits_truncate(
                u32::from_str_radix(&mask, 8).unwrap(),
            ));
            return true;
        }
        let module = module_path!().split_once("::").unwrap().1;
        let test = format!("{module}::{test}");
        let output = Command::new(env::current_exe().unwrap())
            .args(["--exact", &test, "--test-threads=1"])
            .env(UMASK_ENV, "0000")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{test}: {stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        false
    }

    /// Every folder and file under `root`, with its mode, by its path relative to `root`, and
    /// whether it is a folder.
    fn modes_under(root: &Path) -> Vec<(String, bool, u32)> {
        let mut found = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let metadata = fs::metadata(&path).unwrap();
                let relative = path.strip_prefix(root).unwrap().display().to_string();
                found.push((
                    relative,
                    metadata.is_dir(),
                    metadata.permissions().mode() & 0o7777,
                ));
                if metadata.is_dir() {
                    pending.push(path);
                }
            }
        }
        found.sort();
        found
    }

    /// Assert that every folder under `root` and `root` itself has `dir_mode`, and every file,
    /// config media among them, has `file_mode`.
    fn assert_modes(root: &Path, dir_mode: u32, file_mode: u32) {
        let root_mode = fs::metadata(root).unwrap().permissions().mode() & 0o7777;
        assert_eq!(root_mode, dir_mode, "{}", root.display());
        for (path, is_dir, mode) in modes_under(root) {
            if is_dir {
                assert_eq!(mode, dir_mode, "folder {path}: {mode:o}");
            } else {
                assert_eq!(mode, file_mode, "file {path}: {mode:o}");
            }
        }
    }

    #[test]
    fn config_privacy_001_a_vms_boot_files_are_private_under_a_permissive_umask() {
        if !under_a_permissive_umask(
            "config_privacy_001_a_vms_boot_files_are_private_under_a_permissive_umask",
        ) {
            return;
        }
        let lab = ZtpLab::new();
        let images = tempdir().unwrap();
        let blank_disks = images.path().join(SHERPA_BLANK_DISK_DIR);
        fs::create_dir(&blank_disks).unwrap();
        fs::write(blank_disks.join(SHERPA_BLANK_DISK_EXT4_500MB), b"disk").unwrap();
        let lab_ztp = LabZtp {
            images_dir: images.path().to_str().unwrap(),
            ..lab.lab()
        };
        // cloud-init, CD-ROM, the PAN-OS bootstrap, NoCloud and Ignition.
        let nodes = [
            ("ubuntu01", data::NodeModel::UbuntuLinux),
            ("cat01", data::NodeModel::CiscoCat8000),
            ("panos01", data::NodeModel::PaloaltoPanos),
            ("talos01", data::NodeModel::TalosLinux),
            ("flatcar01", data::NodeModel::FlatcarLinux),
        ];
        for (name, model) in nodes {
            let mut node = named_node(name, model);
            generate_vm_ztp(
                &mut node,
                &data::ModelKind::default_for(model).node_config(),
                &lab_ztp,
                "192.0.2.10".parse().unwrap(),
                None,
                None,
            )
            .unwrap();
        }

        let boot_dir = lab.dir.path().join(VM_BOOT_DIR);
        assert_modes(&boot_dir, NODE_CONFIG_DIR_MODE, NODE_CONFIG_FILE_MODE);
        let written = modes_under(&boot_dir);
        for (name, _) in nodes {
            assert!(
                written
                    .iter()
                    .any(|(path, is_dir, _)| !is_dir && path.starts_with(&format!("{name}/"))),
                "no config for {name}: {written:?}"
            );
        }
    }

    #[test]
    fn config_privacy_002_network_boot_files_are_private_under_a_permissive_umask() {
        if !under_a_permissive_umask(
            "config_privacy_002_network_boot_files_are_private_under_a_permissive_umask",
        ) {
            return;
        }
        let lab = ZtpLab::new();
        let ztp_dir = lab.dir.path().join(ZTP_DIR);
        // `up` and `redeploy` make the TFTP folder before any node's config.
        util::create_dir_with_permissions(&ztp_dir.join(TFTP_DIR), TFTP_DIR_MODE).unwrap();
        for (name, model) in [
            ("veos01", data::NodeModel::AristaEos),
            ("sonic01", data::NodeModel::SonicLinux),
        ] {
            let mut node = named_node(name, model);
            generate_vm_ztp(
                &mut node,
                &data::ModelKind::default_for(model).node_config(),
                &lab.lab(),
                "192.0.2.10".parse().unwrap(),
                None,
                None,
            )
            .unwrap();
        }

        let tftp = modes_under(&ztp_dir.join(TFTP_DIR));
        assert_eq!(
            tftp,
            [("veos01.conf".to_owned(), false, TFTP_CONFIG_FILE_MODE)]
        );
        let configs_dir = ztp_dir.join(NODE_CONFIGS_DIR);
        assert_modes(&configs_dir, NODE_CONFIG_DIR_MODE, NODE_CONFIG_FILE_MODE);
        assert_eq!(
            modes_under(&configs_dir).len(),
            http_ztp_files("sonic01").len()
        );
        let ztp_dir_mode = fs::metadata(&ztp_dir).unwrap().permissions().mode() & 0o7777;
        assert_eq!(ztp_dir_mode, NODE_CONFIG_DIR_MODE);
    }

    #[test]
    fn device_accounts_023_served_configs_carry_only_a_hash_of_the_password() {
        let served = ModelKind::all().filter_map(|model_kind| {
            let method = model_kind
                .ztp_methods()
                .iter()
                .copied()
                .find(|method| matches!(method, Ztp::Tftp | Ztp::Http))?;
            (model_kind.kind() == data::NodeKind::VirtualMachine).then_some((model_kind, method))
        });
        let mut models = Vec::new();
        for (model_kind, method) in served {
            let lab = ZtpLab::new();
            let ztp_dir = lab.dir.path().join(ZTP_DIR);
            util::create_dir_with_permissions(&ztp_dir.join(TFTP_DIR), TFTP_DIR_MODE).unwrap();
            let model = model_kind.model();
            let mut node = named_node("node01", model);
            node.model_kind = model_kind;
            node.ztp.method = Some(method);
            generate_vm_ztp(
                &mut node,
                &model_kind.node_config(),
                &lab.lab(),
                "192.0.2.10".parse().unwrap(),
                None,
                None,
            )
            .unwrap();

            let plain = lab.account.device_password.expose();
            let mut configs = String::new();
            for dir in [TFTP_DIR, NODE_CONFIGS_DIR] {
                let Ok(entries) = fs::read_dir(ztp_dir.join(dir)) else {
                    continue;
                };
                for entry in entries {
                    configs.push_str(&fs::read_to_string(entry.unwrap().path()).unwrap());
                }
            }
            assert!(!configs.is_empty(), "{model}: nothing is served");
            assert!(!configs.contains(plain), "{model}: {configs}");
            let hashed = configs.contains("$6$") || configs.contains("$5$");
            assert_eq!(
                hashed,
                model != data::NodeModel::ArubaAoscx,
                "{model}: {configs}"
            );
            models.push(model);
        }
        assert!(
            models.contains(&data::NodeModel::ArubaAoscx)
                && models.contains(&data::NodeModel::SonicLinux),
            "{models:?}"
        );
    }

    #[test]
    fn config_privacy_003_container_configs_are_private_under_a_permissive_umask() {
        if !under_a_permissive_umask(
            "config_privacy_003_container_configs_are_private_under_a_permissive_umask",
        ) {
            return;
        }
        let lab = ZtpLab::new();
        let mut nodes = vec![
            named_container("ceos01", data::NodeModel::AristaEos),
            named_node("srl01", data::NodeModel::NokiaSrlinux),
            named_container("cevo01", data::NodeModel::JuniperEvolved),
            named_node("frr01", data::NodeModel::FrrLinux),
            iol_node("iol01", data::NodeModel::CiscoIol, 1),
        ];
        let mut vault = named_node("vault01", data::NodeModel::HashicorpVault);
        vault.ztp.config = Some("ui = true".to_owned());
        nodes.push(vault);
        for node in &mut nodes {
            lab.container(node, None).unwrap();
        }

        for node in &nodes {
            let dir = lab.dir.path().join(&node.name);
            assert_modes(&dir, NODE_CONFIG_DIR_MODE, NODE_CONFIG_FILE_MODE);
            assert!(
                modes_under(&dir).iter().any(|(_, is_dir, _)| !is_dir),
                "no config for {}",
                node.name
            );
        }
    }
}
