//! Per-node operations shared by `up`, `redeploy` and `resume`: configuration, domains and
//! starting nodes.

use std::collections::HashSet;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use askama::Template;
use jato::{Auth, ConnectOptions, Hop, HostKeyPolicy, Node, Session};
use tracing::instrument;

use crate::services::container_networks::is_node_network;
use crate::services::guest_account::{self, GuestAccount, KeyFormat};
use crate::services::image_lifecycle::ImageLease;
use crate::services::progress::ProgressSender;
use crate::tls::generator::write_file_with_permissions;
use crate::tls::lab_ca::NodeCertPaths;

use container::ContainerSpec;
use iso::create_iso;
use network::NetnsVethState;
use shared::data;
use shared::data::StatusKind;
use shared::konst::{
    ARISTA_ABOOT_DIR, ARISTA_CEOS_STARTUP_CONFIG, ARISTA_CEOS_ZTP_VOLUME_MOUNT,
    CISCO_ASAV_ZTP_CONFIG, CISCO_FTDV_ZTP_CONFIG, CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_ZTP_CONFIG,
    CISCO_IOSXR_ZTP_CONFIG, CISCO_ISE_ZTP_CONFIG, CISCO_NXOS_ZTP_CONFIG, CLOUD_INIT_META_DATA,
    CLOUD_INIT_NETWORK_CONFIG, CLOUD_INIT_USER_DATA, CONTAINER_ARISTA_CEOS_COMMANDS,
    CONTAINER_ARISTA_CEOS_ENV_VARS, CONTAINER_DISK_NAME, CONTAINER_FORGEJO_COMMANDS,
    CONTAINER_FRR_ENV_VARS, CONTAINER_GITLAB_CE_COMMANDS, CONTAINER_GITLAB_CE_SHM_SIZE,
    CONTAINER_HASHICORP_VAULT_CAPABILITIES, CONTAINER_HASHICORP_VAULT_COMMANDS,
    CONTAINER_HASHICORP_VAULT_ENV_VARS, CONTAINER_KEYCLOAK_COMMANDS, CONTAINER_KEYCLOAK_USER,
    CONTAINER_MONGO_DB_COMMANDS, CONTAINER_NATS_COMMANDS, CONTAINER_NOKIA_SRLINUX_COMMANDS,
    CONTAINER_NOKIA_SRLINUX_ENV_VARS, CONTAINER_NOKIA_SRLINUX_USER, CONTAINER_SURREAL_DB_COMMANDS,
    CUMULUS_ZTP, FRR_ZTP_CONFIG_MOUNT, FRR_ZTP_DAEMONS_MOUNT, FRR_ZTP_STARTUP_MOUNT,
    JUNIPER_CEVOLVED_CONFIG_DIR, JUNIPER_ZTP_CONFIG, JUNIPER_ZTP_CONFIG_TGZ, KEYCLOAK_HTTPS_PORT,
    KVM_OUI, MIKROTIK_CHR_ZTP_CONFIG, NODE_CERTS_DIR, NODE_CERTS_DIR_WINDOWS,
    NODE_CLOUD_INIT_SUDO_RULE, NODE_CONFIGS_DIR, NODE_DOAS_CONF_FILE, NODE_NIXOS_SHELL,
    NODE_PASSWD_FILE, NODE_STAGING_DIR, NODE_STARTUP_SCRIPTS_DIR, NODE_USER_ENV_FILE,
    NODE_USER_SCRIPTS_DIR, NOKIA_SRLINUX_ZTP_VOLUME_MOUNT, PALOALTO_BOOTSTRAP_CONFIG,
    PALOALTO_BOOTSTRAP_VOLUME_ID, PALOALTO_ZTP_CONFIG, SHERPA_BLANK_DISK_DIR,
    SHERPA_BLANK_DISK_EXT4_500MB, SHERPA_BLANK_DISK_FAT32, SHERPA_BLANK_DISK_IOSV,
    SHERPA_BLANK_DISK_ISE, SHERPA_BLANK_DISK_JUNOS, SHERPA_DOMAIN_NAME,
    SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX, SHERPA_ISOLATED_NETWORK_NAME, SHERPA_PASSWORD,
    SHERPA_PASSWORD_HASH, SHERPA_PASSWORD_HASH_SHA256, SHERPA_RESERVED_NETWORK_BRIDGE_PREFIX,
    SHERPA_RESERVED_NETWORK_NAME, SHERPA_SSH_PUBLIC_KEY_FILE, SHERPA_STORAGE_POOL,
    SHERPA_STORAGE_POOL_PATH, SHERPA_USERNAME, SSH_PORT, SSH_READY_HANDSHAKE_TIMEOUT,
    TALOS_API_PORT, TELNET_PORT, TFTP_DIR, VAULT_API_PORT, VAULT_CLUSTER_PORT,
    VAULT_ZTP_CONFIG_MOUNT, WINDOWS_ADMINISTRATORS_SID, WINDOWS_AUTHORIZED_KEYS_DIR,
    WINDOWS_SSHD_AUTHORIZED_KEYS_DIR, WINDOWS_SSHD_CONFIG, WINDOWS_SYSTEM_SID, ZTP_DIR, ZTP_ISO,
    ZTP_ISO_VOLUME_ID, ZTP_JSON,
};
use shared::util;
use shared::validators::{
    effective_data_interface_count, effective_ztp, mode_from_octal_digits, validate_ssh_key,
};
use topology::{BinaryFileData, SystemdUnitData};
use virt::error::ErrorNumber;
use virt::sys::VIR_DOMAIN_UNDEFINE_NVRAM;

/// The folder in a node's lab folder that holds its binary files before they go on its disk.
const BINARY_FILES_DIR: &str = "binary_files";

// ============================================================================
// Result structs
/// What generating a lab's node configurations needs: its folders and management network, the
/// accounts, and where progress goes.
#[derive(Clone, Copy)]
pub(crate) struct LabZtp<'a> {
    pub(crate) lab_id: &'a str,
    pub(crate) lab_dir: &'a str,
    pub(crate) images_dir: &'a str,
    pub(crate) sherpa_user: &'a data::User,
    pub(crate) dns: &'a data::Dns,
    pub(crate) mgmt_net: &'a data::SherpaNetwork,
    pub(crate) progress: &'a ProgressSender,
    /// The lab owner's account; each node gets its own form of it, if it takes one.
    pub(crate) owner_account: Option<&'a GuestAccount>,
}

/// One VM's part of generating its configuration: the lab's, plus the node's addresses,
/// certificate and account. Every ZTP method's generator takes it.
#[derive(Clone, Copy)]
struct NodeZtp<'a> {
    lab_dir: &'a str,
    tftp_dir: &'a str,
    images_dir: &'a str,
    sherpa_user: &'a data::User,
    dns: &'a data::Dns,
    mgmt_net: &'a data::SherpaNetwork,
    progress: &'a ProgressSender,
    node_ipv4_address: Ipv4Addr,
    mac_address: &'a str,
    node_name_with_lab: &'a str,
    cert_paths: Option<&'a NodeCertPaths>,
    guest_account: Option<&'a GuestAccount>,
}

/// What a container model's setup needs: the node's folder and custom config, the lab's
/// accounts and management network, and the node's address and certificate.
#[derive(Clone, Copy)]
struct ContainerZtp<'a> {
    dir: &'a str,
    custom_ztp: Option<&'a str>,
    sherpa_user: &'a data::User,
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
    pub qemu_commands: Vec<data::QemuCommand>,
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
    pub loopback_ipv4: String,
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
    util::create_dir(dir)?;

    let meta_data = template::MetaDataConfig {
        instance_id: format!("iid-{hostname}"),
        local_hostname: hostname.to_owned(),
        ..Default::default()
    };
    util::create_file(
        &format!("{dir}/{CLOUD_INIT_META_DATA}"),
        meta_data.render()?,
    )?;

    let network_config = template::CloudInitNetwork::ztp_interface(
        node_ipv4_address,
        mac_address.to_owned(),
        &mgmt_net.v4,
        node_ipv6_address,
        mgmt_net.v6.as_ref(),
    );
    util::create_file(
        &format!("{dir}/{CLOUD_INIT_NETWORK_CONFIG}"),
        network_config.render()?,
    )?;

    let user_data_path = format!("{dir}/{CLOUD_INIT_USER_DATA}");
    match user_data {
        Some(content) => {
            write_file_with_permissions(Path::new(&user_data_path), content.as_bytes(), 0o600)?;
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
        lab_dir,
        mgmt_net,
        node_ipv4_address,
        mac_address,
        progress,
        ..
    } = *ztp;
    let dir = format!("{lab_dir}/{}", node.name);
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
pub fn ztp_config_filename(model: data::NodeModel) -> Result<String> {
    let name = match model {
        data::NodeModel::CiscoCsr1000v
        | data::NodeModel::CiscoCat8000v
        | data::NodeModel::CiscoCat9000v => CISCO_IOSXE_ZTP_CONFIG.replace('-', "_"),
        data::NodeModel::CiscoAsav => CISCO_ASAV_ZTP_CONFIG.to_string(),
        data::NodeModel::CiscoNexus9300v => CISCO_NXOS_ZTP_CONFIG.to_string(),
        data::NodeModel::CiscoIosxrv9000 => CISCO_IOSXR_ZTP_CONFIG.to_string(),
        data::NodeModel::CiscoFtdv => CISCO_FTDV_ZTP_CONFIG.to_string(),
        data::NodeModel::JuniperVsrxv3
        | data::NodeModel::JuniperVrouter
        | data::NodeModel::JuniperVswitch => JUNIPER_ZTP_CONFIG.to_string(),
        data::NodeModel::PaloaltoPanos => PALOALTO_ZTP_CONFIG.to_string(),
        _ => bail!("No ZTP config filename mapping for model: {model}"),
    };
    Ok(name)
}

/// If the node has a custom ZTP config, log and send a progress message, then return the content.
pub fn take_custom_ztp_config(
    node: &mut topology::NodeExpanded,
    progress: &ProgressSender,
) -> Option<String> {
    match node.ztp_config.take() {
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
pub fn model_capabilities(model: data::NodeModel) -> Vec<String> {
    match model {
        data::NodeModel::HashicorpVault => CONTAINER_HASHICORP_VAULT_CAPABILITIES
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
        sherpa_user,
        dns,
        mgmt_net,
        progress,
        owner_account: guest_account,
        ..
    } = *lab;
    let node_account = guest_account::node_owner(guest_account, node, node_image)?;
    let dir = format!("{}/{}", lab_dir, node.name);
    let custom_ztp = take_custom_ztp_config(node, progress);
    let ztp = ContainerZtp {
        dir: &dir,
        custom_ztp: custom_ztp.as_deref(),
        sherpa_user,
        guest_account: node_account.as_ref(),
        dns,
        mgmt_net,
        node_ipv4_address,
        cert_paths,
    };
    let mut setup = ContainerSetup {
        image: node.image.clone().unwrap_or_default(),
        env_vars: node.environment_variables.clone().unwrap_or_default(),
        volumes: Vec::new(),
        commands: node.commands.clone().unwrap_or_default(),
        capabilities: Vec::new(),
        privileged: node.privileged.unwrap_or(false),
        shm_size: node.shm_size,
        user: node.user.clone(),
    };

    let setting = effective_ztp(node.ztp, node_image);
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
    match node.model {
        data::NodeModel::AristaCeos => arista_ceos_start(node, setup),
        data::NodeModel::NokiaSrlinux => srlinux_start(node, setup),
        data::NodeModel::JuniperCevolved => cevolved_start(node, node_image, setup)?,
        data::NodeModel::FrrLinux => frr_start(node, setup),
        data::NodeModel::GitlabCe => {
            app_container(node, setup, CONTAINER_GITLAB_CE_COMMANDS);
            // GitLab CE requires a larger /dev/shm for bundled PostgreSQL
            if setup.shm_size.is_none() {
                setup.shm_size = Some(CONTAINER_GITLAB_CE_SHM_SIZE * 1024 * 1024);
            }
            node.shm_size = setup.shm_size;
        }
        data::NodeModel::SurrealDb => app_container(node, setup, CONTAINER_SURREAL_DB_COMMANDS),
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
    match node.model {
        data::NodeModel::AristaCeos => arista_ceos_volume_ztp(node, ztp, setup),
        data::NodeModel::NokiaSrlinux => srlinux_volume_ztp(node, ztp, setup),
        data::NodeModel::JuniperCevolved => cevolved_volume_ztp(node, node_image, ztp, setup),
        data::NodeModel::FrrLinux => frr_volume_ztp(node, ztp, setup),
        model => bail!(
            "ZTP setting 'volume' of container node '{}' was not validated: Sherpa generates \
             no config for model {model}",
            node.name
        ),
    }
}

/// The manifest's mounts and the node certificate mounts. The manifest's are dropped for a
/// node whose config Sherpa mounts (`volume`), and for Vault, GitLab CE and `SurrealDB`. A node
/// with ZTP off keeps them: they are how its user mounts a config of their own.
fn add_container_volumes(
    node: &topology::NodeExpanded,
    setting: data::Ztp,
    cert_paths: Option<&NodeCertPaths>,
    volumes: &mut Vec<String>,
) {
    let drops_manifest_volumes = setting == data::Ztp::Volume
        || matches!(
            node.model,
            data::NodeModel::GitlabCe
                | data::NodeModel::SurrealDb
                | data::NodeModel::HashicorpVault
        );
    if let Some(node_volumes) = &node.volumes
        && !drops_manifest_volumes
    {
        *volumes = node_volumes
            .iter()
            .map(|v| format!("{}:{}", v.src, v.dst))
            .collect();
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
fn arista_ceos_start(node: &mut topology::NodeExpanded, setup: &mut ContainerSetup) {
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
    node.privileged = Some(setup.privileged);
    node.environment_variables = Some(setup.env_vars.clone());
    node.commands = Some(setup.commands.clone());
}

/// cEOS ZTP: its startup config, mounted read-only and copied into flash on first boot.
fn arista_ceos_volume_ztp(
    node: &mut topology::NodeExpanded,
    ztp: &ContainerZtp<'_>,
    setup: &mut ContainerSetup,
) -> Result<()> {
    let ContainerZtp {
        dir,
        custom_ztp,
        sherpa_user,
        guest_account,
        dns,
        mgmt_net,
        node_ipv4_address,
        ..
    } = *ztp;
    let rendered_template = if let Some(config) = custom_ztp {
        config.to_owned()
    } else {
        template::AristaCeosZtpTemplate {
            hostname: node.name.clone(),
            user: sherpa_user.clone(),
            owner: guest_account::owner_user(guest_account, KeyFormat::OpenSsh)?,
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
    util::create_dir(dir)?;
    util::create_file(&ztp_config, rendered_template)?;

    setup.volumes = vec![format!("{}:{}:ro", ztp_volume.src, ztp_volume.dst)];
    setup.commands = ceos_startup_commands();

    node.volumes = Some(vec![ztp_volume]);
    node.commands = Some(setup.commands.clone());
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
    node.privileged = Some(setup.privileged);
    node.user.clone_from(&setup.user);
    node.environment_variables = Some(setup.env_vars.clone());
    node.commands = Some(setup.commands.clone());
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
        sherpa_user,
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
            sherpa_user,
            guest_account::owner_user(guest_account, KeyFormat::OpenSsh)?.as_ref(),
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
    util::create_dir(dir)?;
    util::create_file(&ztp_config, srlinux_config)?;

    setup.volumes = vec![format!("{}:{}", ztp_volume.src, ztp_volume.dst)];
    node.volumes = Some(vec![ztp_volume]);
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
        effective_data_interface_count(&node.name, node.data_interface_count, node_image)?;
    setup.privileged = true;
    setup.env_vars = cevolved_env_vars(
        node.platform.unwrap_or_default(),
        data_interfaces,
        node.cpu_count.unwrap_or(node_image.compute.cpu_count),
        node.memory.unwrap_or(node_image.compute.memory),
    );

    // Update node fields for consistency
    node.privileged = Some(setup.privileged);
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
        sherpa_user,
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
            user: sherpa_user.clone(),
            owner: guest_account::owner_user(guest_account, KeyFormat::Junos)?,
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
    util::create_dir(&config_dir)?;
    util::create_file(
        &format!("{config_dir}/{JUNIPER_ZTP_CONFIG}"),
        rendered_template,
    )?;

    setup.volumes = vec![format!("{}:{}:ro", config_volume.src, config_volume.dst)];
    node.volumes = Some(vec![config_volume]);
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

    node.privileged = Some(setup.privileged);
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
        sherpa_user,
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
        user: sherpa_user.clone(),
        owner: guest_account::owner_user(guest_account, KeyFormat::OpenSsh)?,
    }
    .render()?;
    let startup_file = format!("{dir}/sherpa-init.sh");

    util::create_dir(dir)?;
    util::create_file(&daemons_file, rendered_daemons)?;
    util::create_file(&config_file, rendered_config)?;
    util::create_file(&startup_file, rendered_startup)?;

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

    node.volumes = Some(vec![daemons_volume, config_volume, startup_volume]);
    node.commands = Some(setup.commands.clone());
    Ok(())
}

/// An application container: its model's command.
fn app_container(node: &mut topology::NodeExpanded, setup: &mut ContainerSetup, commands: &[&str]) {
    setup.commands = commands
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    node.commands = Some(setup.commands.clone());
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
    util::create_dir(dir)?;
    util::create_file(&config_file, rendered_config)?;

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
    setup.capabilities = model_capabilities(node.model);

    node.environment_variables = Some(setup.env_vars.clone());
    node.volumes = Some(vec![config_volume]);
    node.commands = Some(setup.commands.clone());
    Ok(())
}

/// Keycloak: HTTPS only, with the lab certificate, run as its own user.
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
    setup.user = Some(CONTAINER_KEYCLOAK_USER.to_string());
    setup.env_vars = merge_env_vars(
        keycloak_env_vars(ztp.node_ipv4_address),
        node.environment_variables.as_deref().unwrap_or_default(),
    );
    setup.commands = CONTAINER_KEYCLOAK_COMMANDS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    node.environment_variables = Some(setup.env_vars.clone());
    node.commands = Some(setup.commands.clone());
    node.user.clone_from(&setup.user);
    Ok(())
}

/// How a P2p container's data interface veth comes up: a disabled one stays down, unless the
/// model keeps every data interface up, when only its host end is down.
pub(crate) fn netns_veth_state(model: data::NodeModel, admin_down: bool) -> NetnsVethState {
    if !admin_down {
        NetnsVethState::Enabled
    } else if model.keeps_data_interfaces_up() {
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
        sherpa_user,
        dns,
        mgmt_net,
        progress,
        owner_account: guest_account,
    } = *lab;
    let tftp_dir = format!("{lab_dir}/{ZTP_DIR}/{TFTP_DIR}");
    let node_account = guest_account::node_owner(guest_account, node, node_image)?;
    let guest_account = node_account.as_ref();
    let node_name_with_lab = format!("{}-{}", node.name, lab_id);

    // Reuse existing MAC address if provided, otherwise generate a new one
    let mac_address = mgmt_mac.map_or_else(
        || util::random_mac(KVM_OUI),
        std::string::ToString::to_string,
    );

    let ztp = NodeZtp {
        lab_dir,
        tftp_dir: &tftp_dir,
        images_dir,
        sherpa_user,
        dns,
        mgmt_net,
        progress,
        node_ipv4_address,
        mac_address: &mac_address,
        node_name_with_lab: &node_name_with_lab,
        cert_paths,
        guest_account,
    };
    let setting = effective_ztp(node.ztp, node_image);
    let media = generate_vm_ztp_media(node, node_image, &ztp, setting)?;
    // The image's own CD-ROM, such as vEOS's Aboot, unless the ZTP method brings one.
    let image_cdrom = node_image.disks.cdrom.as_ref().map(|src_iso| {
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
    let qemu_commands = vm_qemu_commands(
        node_image.model,
        media.ignition.as_ref().map(|(_, dst)| dst.as_str()),
    );

    let ztp_record = data::ZtpRecord {
        node_name: node.name.clone(),
        config_file: format!("{}.conf", node.name),
        ipv4_address: node_ipv4_address,
        ipv6_address: None,
        mac_address: mac_address.clone(),
        ztp: setting,
        ssh_port: node_image.model.has_ssh().then_some(SSH_PORT),
    };

    Ok(VmZtpResult {
        ztp_record,
        clone_disks,
        disks,
        mac_address,
        qemu_commands,
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
        lab_dir,
        progress,
        node_name_with_lab,
        ..
    } = *ztp;
    let custom_ztp = take_custom_ztp_config(node, progress);
    let custom_ztp = custom_ztp.as_deref();
    let ztp_iso = (
        format!("{lab_dir}/{}/{ZTP_ISO}", node.name),
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
    let hdd_bus = &node_image.disks.hdd_bus;
    let cdrom_bus = &node_image.disks.cdrom_bus;
    let filename = util::image_filename(&node_image.kind, node_image.compute.boot_mode.as_ref());
    let dst_boot_disk = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-hdd.qcow2");
    let mut clone_disks = vec![data::CloneDisk {
        src: format!(
            "{}/{}/{}/{}",
            images_dir, node_image.model, node_image.version, filename
        ),
        dst: dst_boot_disk.clone(),
        disk_size: node.boot_disk_size,
        method: node.boot_disk_clone_method.unwrap_or_default(),
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

    // The Ignition config is read by QEMU, not attached as a disk.
    if let Some((src, dst)) = &media.ignition {
        clone_disks.push(copied_disk(src, dst));
    }
    Ok((clone_disks, disks))
}

/// The extra QEMU arguments of a VM model; Flatcar's point it at its Ignition config.
fn vm_qemu_commands(model: data::NodeModel, ignition_dst: Option<&str>) -> Vec<data::QemuCommand> {
    match model {
        data::NodeModel::JuniperVrouter => data::QemuCommand::juniper_vrouter(),
        data::NodeModel::JuniperVswitch => data::QemuCommand::juniper_vswitch(),
        data::NodeModel::JuniperVsrxv3 => data::QemuCommand::juniper_vsrxv3(),
        data::NodeModel::JuniperVevolved => data::QemuCommand::juniper_vevolved(),
        data::NodeModel::FlatcarLinux => {
            ignition_dst.map_or_else(Vec::new, data::QemuCommand::ignition_config)
        }
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
/// only on the node's address with the lab certificate, and the Sherpa user as the bootstrap
/// admin of the `master` realm.
fn keycloak_env_vars(node_address: Ipv4Addr) -> Vec<String> {
    vec![
        "KC_DB=dev-file".to_owned(),
        "KC_HTTP_ENABLED=false".to_owned(),
        format!("KC_HOSTNAME=https://{node_address}:{KEYCLOAK_HTTPS_PORT}"),
        format!("KC_HTTPS_CERTIFICATE_FILE={NODE_CERTS_DIR}/node.crt"),
        format!("KC_HTTPS_CERTIFICATE_KEY_FILE={NODE_CERTS_DIR}/node.key"),
        "KC_HEALTH_ENABLED=true".to_owned(),
        format!("KC_BOOTSTRAP_ADMIN_USERNAME={SHERPA_USERNAME}"),
        format!("KC_BOOTSTRAP_ADMIN_PASSWORD={SHERPA_PASSWORD}"),
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

/// cloud-init files and runcmd lines for a node's scripts: `startup_scripts` run as root, then
/// `user_scripts` run as `user` in a login shell. Each script is run directly, so its shebang
/// picks the interpreter and nodes without `bash` can run them.
fn build_cloud_init_scripts(
    node_name: &str,
    startup_scripts: Option<&[topology::StartupScript]>,
    user_scripts: Option<&[topology::StartupScript]>,
    user: &str,
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

    // Files are written as root (write_files runs before users are created), then chowned
    // once the user exists.
    let user_scripts = user_scripts.unwrap_or_default();
    for script in user_scripts {
        let contents = util::base64_decode(&script.content).with_context(|| {
            format!(
                "Failed to decode user_script '{}' for node '{}'",
                script.filename, node_name
            )
        })?;
        write_files.push(template::CloudInitWriteFile {
            path: format!("{}/{}", NODE_USER_SCRIPTS_DIR, script.filename),
            content: contents,
            permissions: "0755".to_string(),
            owner: None,
            encoding: None,
        });
    }
    if !user_scripts.is_empty() {
        // The user's primary group: SUSE has no group named after the user.
        let quoted_user = shell_quote(user);
        runcmd.push(format!(
            r#"chown -R {quoted_user}:"$(id -gn {quoted_user})" {}"#,
            shell_quote(NODE_USER_SCRIPTS_DIR)
        ));
        for script in user_scripts {
            let target_path = format!("{}/{}", NODE_USER_SCRIPTS_DIR, script.filename);
            runcmd.push(format!(
                "su - {quoted_user} -c {}",
                shell_quote(&shell_quote(&target_path))
            ));
        }
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

/// Build PowerShell commands to set machine-level environment variables on Windows.
fn build_windows_env_commands(env_vars: &[String]) -> Vec<String> {
    let mut commands = Vec::new();
    for entry in env_vars {
        if let Some((key, value)) = entry.split_once('=') {
            // Escape single quotes for PowerShell: ' → ''
            let escaped_value = value.replace('\'', "''");
            commands.push(format!(
                r#"powershell -Command "[System.Environment]::SetEnvironmentVariable('{key}', '{escaped_value}', 'Machine')""#
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
            bail!(
                r#"Windows file path '{}' contains '"' or '%', which cannot be used in a path on a Windows node"#,
                file.path
            );
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
fn cloud_init_ipv6_address(model: data::NodeModel, address: Option<Ipv6Addr>) -> Option<Ipv6Addr> {
    match model {
        data::NodeModel::OpenBsd => None,
        _ => address,
    }
}

/// Whether cloud-init should manage a node's `/etc/hosts`.
///
/// OpenBSD does not: cloud-init ships no hosts template for it, so the module fails and
/// cloud-init reports an error. The node's name resolves through the lab DNS instead.
fn cloud_init_manage_etc_hosts(model: data::NodeModel) -> bool {
    !matches!(model, data::NodeModel::OpenBsd)
}

/// The doas rule giving `wheel` admin rights on OpenBSD, where doas is the native tool: the
/// stock `/etc/examples/doas.conf` rule, without a password, since the logins have none. The
/// image has no `/etc/doas.conf`.
fn cloud_init_doas_file(model: data::NodeModel) -> Option<template::CloudInitWriteFile> {
    if model != data::NodeModel::OpenBsd {
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
fn cloud_init_sudo_rule(model: data::NodeModel) -> Option<String> {
    match model {
        data::NodeModel::OpenBsd | data::NodeModel::NixosLinux => None,
        _ => Some(NODE_CLOUD_INIT_SUDO_RULE.to_owned()),
    }
}

/// Whether cloud-init allows SSH password logins, or `None` to leave the SSH server alone. NixOS
/// generates `sshd_config`, which cloud-init cannot edit, and its image allows password logins
/// itself.
fn cloud_init_ssh_pwauth(model: data::NodeModel) -> Option<bool> {
    match model {
        data::NodeModel::NixosLinux => None,
        _ => Some(true),
    }
}

#[instrument(level = "debug")]
fn cloud_init_user_settings(
    model: data::NodeModel,
    os_variant: &data::OsVariant,
) -> (&'static str, &'static str) {
    match (model, os_variant) {
        (data::NodeModel::OmarchyLinux, _) => ("wheel", "/bin/bash"),
        (data::NodeModel::NixosLinux, _) => ("wheel", NODE_NIXOS_SHELL),
        (_, data::OsVariant::Bsd) => ("wheel", "/bin/sh"),
        _ => ("sudo", "/bin/bash"),
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

    let dir = format!("{}/{}", ztp.lab_dir, node.name);
    match node.model {
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
        | data::NodeModel::GenericVm
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
/// startup and user scripts.
fn cloud_init_files(
    node: &topology::NodeExpanded,
    cert_paths: Option<&NodeCertPaths>,
    guest_account: Option<&GuestAccount>,
) -> Result<(Vec<template::CloudInitWriteFile>, Vec<String>)> {
    let mut write_files = match cert_paths {
        Some(certs) => build_cloud_init_cert_files(certs, NODE_CERTS_DIR)?,
        None => vec![],
    };

    // Stage text_files and the guest login's environment file, and move them into
    // place first in runcmd, once their owners exist.
    let guest_username = guest_account::guest_username(guest_account);
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

    // Load the environment file from the login shell, which user_scripts also use.
    if env_vars.is_some() {
        runcmd.push(user_env_loader_runcmd(guest_username, NODE_PASSWD_FILE));
    }

    let (script_files, script_runcmd) = build_cloud_init_scripts(
        &node.name,
        node.startup_scripts.as_deref(),
        node.user_scripts.as_deref(),
        guest_username,
    )?;
    write_files.extend(script_files);
    runcmd.extend(script_runcmd);
    Ok((write_files, runcmd))
}

/// Write a node's cloud-init seed into `dir`, its user data private, and build its ISO.
fn write_cloud_init_seed(
    dir: &str,
    user_data: &str,
    meta_data: String,
    network_config: String,
) -> Result<()> {
    util::create_dir(dir)?;
    write_file_with_permissions(
        Path::new(&format!("{dir}/{CLOUD_INIT_USER_DATA}")),
        user_data.as_bytes(),
        0o600,
    )?;
    util::create_file(&format!("{dir}/{CLOUD_INIT_META_DATA}"), meta_data)?;
    util::create_file(
        &format!("{dir}/{CLOUD_INIT_NETWORK_CONFIG}"),
        network_config,
    )?;
    create_iso(dir, format!("{dir}/{ZTP_ISO}"), ZTP_ISO_VOLUME_ID)?;
    Ok(())
}

/// Cloud-init for the Linux and BSD VMs: the sherpa login and the owner's, with their admin
/// rights, and the node's files and scripts.
fn linux_cloud_init(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
    dir: &str,
) -> Result<()> {
    let NodeZtp {
        sherpa_user,
        mgmt_net,
        node_ipv4_address,
        mac_address,
        cert_paths,
        guest_account,
        ..
    } = *ztp;
    let mut cloud_init_user = template::CloudInitUser::sherpa(&sherpa_user.ssh_public_key);
    let (admin_group, shell) = cloud_init_user_settings(node.model, &node_image.os_variant);
    cloud_init_user.groups = vec![admin_group.to_owned()];
    shell.clone_into(&mut cloud_init_user.shell);
    cloud_init_user.sudo = cloud_init_sudo_rule(node.model);

    let (mut write_files, runcmd) = cloud_init_files(node, cert_paths, guest_account)?;
    if let Some(doas_file) = cloud_init_doas_file(node.model) {
        write_files.push(doas_file);
    }

    let mut users = vec![cloud_init_user];
    if let Some(account) = guest_account {
        let mut user = match (node.model, &node_image.os_variant) {
            (data::NodeModel::NixosLinux, _) => guest_account::nixos_cloud_init_user(account),
            (_, data::OsVariant::Bsd) => guest_account::bsd_cloud_init_user(account),
            _ => guest_account::cloud_init_user(account),
        };
        user.sudo = cloud_init_sudo_rule(node.model);
        users.push(user);
    }

    let cloud_init_config = template::CloudInitConfig {
        ca_certs: cloud_init_ca_certs(cert_paths)?,
        hostname: node.name.clone(),
        fqdn: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
        manage_etc_hosts: cloud_init_manage_etc_hosts(node.model),
        ssh_pwauth: cloud_init_ssh_pwauth(node.model),
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
        cloud_init_ipv6_address(node.model, node.ipv6_address),
        mgmt_net.v6.as_ref(),
    );
    write_cloud_init_seed(
        dir,
        &cloud_init_config.render()?,
        meta_data.render()?,
        network_config.render()?,
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

/// Cloudbase-init for the Windows VMs: the sherpa login and the owner's, each with its own SSH
/// keys file, and the node's files, startup scripts and machine environment.
fn windows_cloudbase_init(
    node: &topology::NodeExpanded,
    ztp: &NodeZtp<'_>,
    dir: &str,
) -> Result<()> {
    let NodeZtp {
        sherpa_user,
        mgmt_net,
        node_ipv4_address,
        mac_address,
        cert_paths,
        guest_account,
        ..
    } = *ztp;
    let ssh_key = &sherpa_user.ssh_public_key;
    let mut users = vec![template::CloudbaseInitUser::sherpa(ssh_key)];
    let ssh_key_str = format!("{} {}", ssh_key.algorithm, ssh_key.key);

    // Every account reads its own keys file, private to Administrators and SYSTEM.
    let sshd_config = template::WindowsSshdConfigTemplate {
        authorized_keys_dir: WINDOWS_SSHD_AUTHORIZED_KEYS_DIR.to_owned(),
    };
    let mut write_files = vec![
        template::CloudbaseWriteFile {
            path: WINDOWS_SSHD_CONFIG.to_owned(),
            content: sshd_config.render()?,
            permissions: "0644".to_string(),
        },
        template::CloudbaseWriteFile {
            path: format!(r"{WINDOWS_AUTHORIZED_KEYS_DIR}\{SHERPA_USERNAME}"),
            content: format!("{ssh_key_str}\n"),
            permissions: "0600".to_string(),
        },
    ];
    if let Some(account) = guest_account {
        users.push(guest_account::cloudbase_user(account));
        write_files.push(template::CloudbaseWriteFile {
            path: format!(r"{WINDOWS_AUTHORIZED_KEYS_DIR}\{}", account.username),
            content: format!("{}\n", account.ssh_keys.join("\n")),
            permissions: "0600".to_string(),
        });
    }
    if let Some(certs) = cert_paths {
        write_files.extend(build_cloudbase_cert_files(certs, NODE_CERTS_DIR_WINDOWS)?);
    }
    write_files.extend(build_cloudbase_text_files(node.text_files.as_deref()));

    let mut runcmd = build_windows_private_file_acls(&write_files)
        .with_context(|| format!("Failed to write files for node '{}'", node.name))?;
    runcmd.push(r#"powershell -Command "Restart-Service sshd""#.to_string());
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
        public_keys: vec![ssh_key_str],
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
        meta_data.render()?,
        network_config.render()?,
    )
}

/// Cloud-init for Alpine: `wheel` with doas, and the lab DNS written to `resolv.conf`.
fn alpine_cloud_init(node: &topology::NodeExpanded, ztp: &NodeZtp<'_>, dir: &str) -> Result<()> {
    let NodeZtp {
        sherpa_user,
        mgmt_net,
        node_ipv4_address,
        mac_address,
        cert_paths,
        guest_account,
        ..
    } = *ztp;
    let mut cloud_init_user = template::CloudInitUser::sherpa(&sherpa_user.ssh_public_key);
    cloud_init_user.shell = "/bin/sh".to_string();
    cloud_init_user.groups = vec!["wheel".to_string()];

    let (mut write_files, runcmd) = cloud_init_files(node, cert_paths, guest_account)?;
    let mut users = vec![cloud_init_user];
    if let Some(account) = guest_account {
        write_files.push(guest_account::doas_file(account));
        users.push(guest_account::alpine_cloud_init_user(account));
    }

    let cloud_init_config = template::CloudInitConfig {
        ca_certs: cloud_init_ca_certs(cert_paths)?,
        hostname: node.name.clone(),
        fqdn: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
        manage_etc_hosts: cloud_init_manage_etc_hosts(node.model),
        ssh_pwauth: cloud_init_ssh_pwauth(node.model),
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
        meta_data.render()?,
        network_config.render()?,
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
        sherpa_user,
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
        util::create_file(&ztp_config, custom_config.to_owned())?;
    } else {
        match node.model {
            data::NodeModel::AristaVeos => {
                let arista_template = template::AristaVeosZtpTemplate {
                    hostname: node.name.clone(),
                    user: sherpa_user.clone(),
                    owner: guest_account::owner_user(guest_account, KeyFormat::OpenSsh)?,
                    dns: dns.clone(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                let rendered_template = arista_template.render()?;
                let ztp_config = format!("{tftp_dir}/{}.conf", node.name);
                util::create_file(&ztp_config, rendered_template)?;
            }
            data::NodeModel::ArubaAoscx => {
                let aruba_template = template::ArubaAoscxTemplate {
                    hostname: node.name.clone(),
                    user: sherpa_user.clone(),
                    owner: guest_account::owner_user(guest_account, KeyFormat::OpenSsh)?,
                    dns: dns.clone(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                let rendered_template = aruba_template.render()?;
                let ztp_config = format!("{tftp_dir}/{}.conf", node.name);
                util::create_file(&ztp_config, rendered_template)?;
            }
            data::NodeModel::JuniperVevolved | data::NodeModel::JuniperVsrxv3 => {
                let juniper_template = template::JunipervJunosZtpTemplate {
                    hostname: node.name.clone(),
                    user: sherpa_user.clone(),
                    owner: guest_account::owner_user(guest_account, KeyFormat::Junos)?,
                    mgmt_interface: node_image.interfaces.management.to_string(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                let juniper_rendered_template = juniper_template.render()?;
                let ztp_config = format!("{tftp_dir}/{}.conf", node.name);
                util::create_file(&ztp_config, juniper_rendered_template)?;
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

    let dir = format!("{}/{}", ztp.lab_dir, node.name);
    let config = match custom_ztp {
        Some(custom_config) => custom_config.to_owned(),
        // PAN-OS boots from a bootstrap layout of its own, not one config file.
        None if node.model == data::NodeModel::PaloaltoPanos => {
            return write_panos_bootstrap(node, ztp, &dir);
        }
        None => cdrom_config(node, node_image, ztp)?,
    };
    let ztp_config = format!("{dir}/{}", ztp_config_filename(node.model)?);
    util::create_dir(&dir)?;
    util::create_file(&ztp_config, config)?;
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
        sherpa_user,
        dns,
        mgmt_net,
        node_ipv4_address,
        guest_account,
        ..
    } = *ztp;
    match node.model {
        data::NodeModel::CiscoCsr1000v
        | data::NodeModel::CiscoCat8000v
        | data::NodeModel::CiscoCat9000v => iosxe_config(node, node_image, ztp),
        data::NodeModel::CiscoAsav => asav_config(node, ztp),
        data::NodeModel::CiscoNexus9300v => Ok(template::CiscoNxosZtpTemplate {
            hostname: node.name.clone(),
            user: sherpa_user.clone(),
            owner: guest_account::owner_user(guest_account, KeyFormat::OpenSsh)?,
            dns: dns.clone(),
            mgmt_ipv4_address: Some(node_ipv4_address),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?),
        data::NodeModel::CiscoIosxrv9000 => Ok(template::CiscoIosxrZtpTemplate {
            hostname: node.name.clone(),
            user: sherpa_user.clone(),
            owner: guest_account::owner_user(guest_account, KeyFormat::OpenSsh)?,
            dns: dns.clone(),
            mgmt_ipv4_address: Some(node_ipv4_address),
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?),
        data::NodeModel::CiscoFtdv => ftdv_config(node, ztp),
        data::NodeModel::JuniperVsrxv3
        | data::NodeModel::JuniperVrouter
        | data::NodeModel::JuniperVswitch => Ok(template::JunipervJunosZtpTemplate {
            hostname: node.name.clone(),
            user: sherpa_user.clone(),
            owner: guest_account::owner_user(guest_account, KeyFormat::Junos)?,
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

/// IOS-XE: the sherpa key as its MD5 hash, and the Cat8000v and Cat9000v boot licences.
fn iosxe_config(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
) -> Result<String> {
    let NodeZtp {
        sherpa_user,
        dns,
        mgmt_net,
        node_ipv4_address,
        guest_account,
        ..
    } = *ztp;
    let license_boot_command = match node.model {
        data::NodeModel::CiscoCat8000v => {
            Some("license boot level network-premier addon dna-premier".to_string())
        }
        data::NodeModel::CiscoCat9000v => {
            Some("license boot level network-advantage addon dna-advantage".to_string())
        }
        _ => None,
    };
    let mut user = sherpa_user.clone();
    user.ssh_public_key.key = util::pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
    Ok(template::CiscoIosXeZtpTemplate {
        hostname: node.name.clone(),
        user,
        owner: guest_account::owner_user(guest_account, KeyFormat::CiscoMd5)?,
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

/// `ASAv`: the sherpa key as its SHA-256 hash.
fn asav_config(node: &topology::NodeExpanded, ztp: &NodeZtp<'_>) -> Result<String> {
    let NodeZtp {
        sherpa_user,
        dns,
        mgmt_net,
        node_ipv4_address,
        guest_account,
        ..
    } = *ztp;
    let mut user = sherpa_user.clone();
    user.ssh_public_key.key = util::pub_ssh_key_to_sha256_hash(&user.ssh_public_key.key)?;
    Ok(template::CiscoAsavZtpTemplate {
        hostname: node.name.clone(),
        user,
        owner: guest_account::owner_user(guest_account, KeyFormat::CiscoSha256)?,
        dns: dns.clone(),
        mgmt_ipv4_address: Some(node_ipv4_address),
        mgmt_ipv4: mgmt_net.v4.clone(),
        mgmt_ipv6_address: node.ipv6_address,
        mgmt_ipv6: mgmt_net.v6.clone(),
    }
    .render()?)
}

/// `FTDv`: its day-0 JSON, managed locally with the sherpa password.
fn ftdv_config(node: &topology::NodeExpanded, ztp: &NodeZtp<'_>) -> Result<String> {
    let NodeZtp {
        mgmt_net,
        node_ipv4_address,
        ..
    } = *ztp;
    Ok(serde_json::to_string(&template::CiscoFtdvZtpTemplate {
        eula: "accept".to_string(),
        hostname: node.name.clone(),
        admin_password: SHERPA_PASSWORD.to_string(),
        dns1: Some(mgmt_net.v4.boot_server),
        ipv4_mode: Some(template::CiscoFxosIpMode::Manual),
        ipv4_addr: Some(node_ipv4_address),
        ipv4_gw: Some(mgmt_net.v4.first),
        ipv4_mask: Some(mgmt_net.v4.subnet_mask),
        manage_locally: template::YesNo(true),
        ..Default::default()
    })?)
}

/// PAN-OS bootstrap: `init-cfg.txt` and `bootstrap.xml` in the layout PA-VM reads, on an ISO
/// with the bootstrap volume label.
fn write_panos_bootstrap(
    node: &topology::NodeExpanded,
    ztp: &NodeZtp<'_>,
    dir: &str,
) -> Result<()> {
    let NodeZtp {
        sherpa_user,
        mgmt_net,
        node_ipv4_address,
        guest_account,
        ..
    } = *ztp;
    let user = sherpa_user.clone();
    // PA-VM requires config/, content/, license/, software/ directories
    let config_subdir = format!("{dir}/config");
    util::create_dir(&config_subdir)?;
    util::create_dir(&format!("{dir}/content"))?;
    util::create_dir(&format!("{dir}/license"))?;
    util::create_dir(&format!("{dir}/software"))?;

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
    util::create_file(
        &format!("{config_subdir}/{PALOALTO_ZTP_CONFIG}"),
        init_cfg.render()?,
    )?;

    // bootstrap.xml — full running config with admin + sherpa users
    // PAN-OS expects the full SSH public key line base64-encoded
    let ssh_full_key = format!(
        "{} {}",
        user.ssh_public_key.algorithm, user.ssh_public_key.key
    );
    let ssh_public_key_b64 = util::base64_encode(&ssh_full_key);
    // PAN-OS stores a password hash, so the owner gets one no password matches.
    let mut owner = guest_account::owner_user(guest_account, KeyFormat::PanosBase64)?;
    if let Some(owner) = owner.as_mut() {
        owner.secret = guest_account::unusable_sha256_crypt_hash();
    }
    let bootstrap = template::PaloAltoPanosBootstrapTemplate {
        hostname: node.name.clone(),
        user,
        owner,
        password_hash: SHERPA_PASSWORD_HASH_SHA256.to_string(),
        ssh_public_key_b64,
        mgmt_ipv4_address: node_ipv4_address,
        mgmt_ipv6_address: node.ipv6_address,
        mgmt_ipv6: mgmt_net.v6.clone(),
        mgmt_netmask: mgmt_net.v4.subnet_mask,
        mgmt_gateway: mgmt_net.v4.first,
        dns_primary: mgmt_net.v4.boot_server,
    };
    util::create_file(
        &format!("{config_subdir}/{PALOALTO_BOOTSTRAP_CONFIG}"),
        bootstrap.render()?,
    )?;

    create_iso(
        dir,
        format!("{dir}/{ZTP_ISO}"),
        PALOALTO_BOOTSTRAP_VOLUME_ID,
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
        lab_dir,
        images_dir,
        node_name_with_lab,
        progress,
        ..
    } = *ztp;
    let _ = progress.send_status(
        format!("Creating Disk ZTP config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let dir = format!("{lab_dir}/{}", node.name);
    match node.model {
        data::NodeModel::CiscoIosv | data::NodeModel::CiscoIosvl2 => {
            let rendered_template = match custom_ztp {
                Some(config) => config.to_owned(),
                None => iosv_config(node, node_image, ztp)?,
            };
            let ztp_config = format!("{dir}/{CISCO_IOSV_ZTP_CONFIG}");
            util::create_dir(&dir)?;
            util::create_file(&ztp_config, rendered_template)?;

            let src_disk = format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_IOSV}");
            let dst_disk = format!("{dir}/{}-cfg.img", node.name);

            util::copy_file(&src_disk, &dst_disk)?;
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

/// `IOSv` and `IOSvL2`: the sherpa key as its MD5 hash.
fn iosv_config(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    ztp: &NodeZtp<'_>,
) -> Result<String> {
    let NodeZtp {
        sherpa_user,
        dns,
        mgmt_net,
        node_ipv4_address,
        guest_account,
        ..
    } = *ztp;
    let mut user = sherpa_user.clone();
    user.ssh_public_key.key = util::pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
    let owner = guest_account::owner_user(guest_account, KeyFormat::CiscoMd5)?;
    let mgmt_interface = node_image.interfaces.management.to_string();
    let rendered = if node.model == data::NodeModel::CiscoIosvl2 {
        template::CiscoIosvl2ZtpTemplate {
            hostname: node.name.clone(),
            user,
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
        template::CiscoIosvZtpTemplate {
            hostname: node.name.clone(),
            user,
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

/// ISE: its config on an ext4 disk. ISE has a single administrator, which takes the owner's key
/// when there is one.
fn ise_config_disk(
    node: &topology::NodeExpanded,
    ztp: &NodeZtp<'_>,
    custom_ztp: Option<&str>,
    dir: &str,
) -> Result<(String, String)> {
    let NodeZtp {
        images_dir,
        sherpa_user,
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
        let mut user = sherpa_user.clone();
        if let Some(owner) = guest_account::owner_user(guest_account, KeyFormat::OpenSsh)?
            && let Some(key) = owner.ssh_keys.first()
        {
            user.ssh_public_key = data::SshPublicKey {
                algorithm: key.kind.parse()?,
                key: key.value.clone(),
                comment: None,
            };
        }
        template::CiscoIseZtpTemplate {
            hostname: node.name.clone(),
            user,
            dns: dns.clone(),
            mgmt_ipv4_address: node_ipv4_address,
            mgmt_ipv4: mgmt_net.v4.clone(),
            mgmt_ipv6_address: node.ipv6_address,
            mgmt_ipv6: mgmt_net.v6.clone(),
        }
        .render()?
    };
    let ztp_config = format!("{dir}/{CISCO_ISE_ZTP_CONFIG}");
    util::create_dir(dir)?;
    util::create_file(&ztp_config, rendered_template)?;

    let src_disk = format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_ISE}");
    let dst_disk = format!("{dir}/{node_name_with_lab}-cfg.img");

    util::copy_file(&src_disk, &dst_disk)?;
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
        sherpa_user,
        dns,
        mgmt_net,
        node_ipv4_address,
        node_name_with_lab,
        guest_account,
        ..
    } = *ztp;
    let owner = guest_account::owner_user(guest_account, KeyFormat::OpenSsh)?;
    let mut owner_key_files = Vec::new();
    let mut owner_key_paths = Vec::new();
    if let Some(owner) = &owner {
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
    }
    let rendered_template = if let Some(config) = custom_ztp {
        config.to_owned()
    } else {
        template::MikrotikRouterosZtpTemplate {
            hostname: node.name.clone(),
            user: sherpa_user.clone(),
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
    util::create_dir(dir)?;
    util::create_file(&ztp_config, rendered_template)?;

    let src_disk = format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_FAT32}");
    let dst_disk = format!("{dir}/{}-cfg.img", node.name);

    util::copy_file(&src_disk, &dst_disk)?;
    util::copy_to_dos_image(&ztp_config, &dst_disk, "/")?;
    let sherpa_key_file = format!("{dir}/{SHERPA_SSH_PUBLIC_KEY_FILE}");
    util::create_file(
        &sherpa_key_file,
        format!(
            "{} {}\n",
            sherpa_user.ssh_public_key.algorithm, sherpa_user.ssh_public_key.key
        ),
    )?;
    util::copy_to_dos_image(&sherpa_key_file, &dst_disk, "/")?;
    for (path, content) in owner_key_files {
        util::create_file(&path, content)?;
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
        lab_dir,
        images_dir,
        sherpa_user,
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

    let dir = format!("{lab_dir}/{}", node.name);
    let user = sherpa_user.clone();

    match node_image.model {
        data::NodeModel::CumulusLinux => {
            let rendered_template = if let Some(config) = custom_ztp {
                config.to_owned()
            } else {
                let t = template::CumulusLinuxZtpTemplate {
                    hostname: node.name.clone(),
                    user,
                    owner: guest_account::owner_user(guest_account, KeyFormat::OpenSsh)?,
                    dns: dns.clone(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                t.render()?
            };
            let ztp_config = format!("{dir}/{CUMULUS_ZTP}");
            util::create_dir(&dir)?;
            util::create_file(&ztp_config, rendered_template)?;

            let src_usb = format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_FAT32}");
            let dst_usb = format!("{dir}/cfg.img");

            util::copy_file(&src_usb, &dst_usb)?;
            util::copy_to_dos_image(&ztp_config, &dst_usb, "/")?;

            let dst_final = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img");
            Ok((dst_usb, dst_final))
        }
        data::NodeModel::JuniperVevolved => {
            let rendered_template = if let Some(config) = custom_ztp {
                config.to_owned()
            } else {
                let t = template::JunipervJunosZtpTemplate {
                    hostname: node.name.clone(),
                    user,
                    owner: guest_account::owner_user(guest_account, KeyFormat::Junos)?,
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

            util::create_dir(&dir)?;
            util::create_file(&ztp_config, rendered_template)?;

            let src_usb = format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_JUNOS}");
            let dst_usb = format!("{dir}/cfg.img");

            util::copy_file(&src_usb, &dst_usb)?;
            util::create_config_archive(&ztp_config, &ztp_config_tgz)?;
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
        sherpa_user,
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

            let ztp_init = format!("{dir}/{}.conf", node.name);
            let sonic_ztp = template::SonicLinuxZtp {
                hostname: node.name.clone(),
                mgmt_ipv4: mgmt_net.v4.clone(),
                mgmt_ipv4_address: Some(node_ipv4_address),
                mgmt_ipv6_address: node.ipv6_address,
                mgmt_ipv6: mgmt_net.v6.clone(),
            };
            let ztp_config = format!("{dir}/{}_config_db.json", node.name);
            util::create_dir(&dir)?;
            util::create_file(&ztp_init, sonic_ztp_file_map)?;
            util::create_file(&ztp_config, sonic_ztp.config())?;

            let sonic_user_template = template::SonicLinuxUserTemplate {
                user: sherpa_user.clone(),
                owner: guest_account::owner_user(guest_account, KeyFormat::OpenSsh)?,
            };
            let ztp_user_script = format!(
                "{dir}/{}",
                template::SonicLinuxZtp::user_script_filename(&node.name)
            );
            util::create_file(&ztp_user_script, sonic_user_template.render()?)?;
        }
        _ => {
            bail!("HTTP ZTP method not supported for {}", node_image.model);
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

/// Write the binary files the client read into `{dir}/binary_files`, and return their paths for
/// copying onto the node's data disk. `check_client_read_files` has checked their names.
fn write_binary_files(dir: &str, files: &[BinaryFileData]) -> Result<Vec<String>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let binary_dir = format!("{dir}/{BINARY_FILES_DIR}");
    util::create_dir(&binary_dir)?;
    files
        .iter()
        .map(|file| {
            let contents = util::base64_decode_bytes(&file.content)
                .with_context(|| format!("binary file '{}' contents", file.filename))?;
            let path = format!("{binary_dir}/{}", file.filename);
            fs::write(&path, contents).with_context(|| format!("Failed to write {path}"))?;
            Ok(path)
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

    let ignition_user = ignition_sherpa_user(node, ztp.sherpa_user)?;
    let mut files = ignition_base_files(&node.name);
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

/// The sherpa login in Ignition, with the sherpa key and the manifest's keys. The client read
/// the key files; the server checks the keys but opens no files.
fn ignition_sherpa_user(
    node: &topology::NodeExpanded,
    user: &data::User,
) -> Result<template::IgnitionUser> {
    let mut authorized_keys = vec![format!(
        "{} {} {}",
        user.ssh_public_key.algorithm,
        user.ssh_public_key.key,
        user.ssh_public_key.comment.clone().unwrap_or_default()
    )];
    let manifest_authorized_key_files = node.ssh_authorized_key_files.clone().unwrap_or_default();
    for key in &manifest_authorized_key_files {
        validate_ssh_key(key)
            .with_context(|| format!("Node '{}' ssh_authorized_key_files", node.name))?;
    }
    authorized_keys.extend(node.ssh_authorized_keys.clone().unwrap_or_default());
    authorized_keys.extend(manifest_authorized_key_files);

    Ok(template::IgnitionUser {
        name: user.username.clone(),
        password_hash: Some(SHERPA_PASSWORD_HASH.to_owned()),
        home_dir: None,
        shell: None,
        ssh_authorized_keys: authorized_keys,
        groups: vec!["wheel".to_owned(), "docker".to_owned()],
    })
}

/// The files every Ignition node gets: the sherpa sudo rule, its hostname, and updates off.
fn ignition_base_files(node_name: &str) -> Vec<template::IgnitionFile> {
    let sudo_config_base64 =
        util::base64_encode(&format!("{SHERPA_USERNAME} ALL=(ALL) NOPASSWD: ALL"));
    let sudo_config_file = template::IgnitionFile {
        path: format!("/etc/sudoers.d/{SHERPA_USERNAME}"),
        mode: template::FileMode::new(0o440),
        contents: Some(template::IgnitionFileContents::new(&format!(
            "data:;base64,{sudo_config_base64}"
        ))),
        ..Default::default()
    };
    let hostname_file = template::IgnitionFile {
        path: "/etc/hostname".to_owned(),
        mode: template::FileMode::new(0o644),
        contents: Some(template::IgnitionFileContents::new(&format!(
            "data:,{node_name}"
        ))),
        ..Default::default()
    };
    vec![
        sudo_config_file,
        hostname_file,
        template::IgnitionFile::disable_updates(),
    ]
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

/// Flatcar: its Ignition config, and a container data disk holding the binary files. Returns
/// the config and the data disk, each in the lab folder and in the storage pool.
fn flatcar_ignition(
    node: &topology::NodeExpanded,
    ztp: &NodeZtp<'_>,
    ignition_user: template::IgnitionUser,
    mut files: Vec<template::IgnitionFile>,
    manifest_systemd_units: Vec<template::IgnitionUnit>,
) -> Result<(String, String, String, String)> {
    let NodeZtp {
        lab_dir,
        images_dir,
        mgmt_net,
        node_ipv4_address,
        node_name_with_lab,
        cert_paths,
        guest_account,
        ..
    } = *ztp;
    let dir = format!("{lab_dir}/{}", node.name);
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

    // Inject environment_variables into the guest login's environment
    if let Some(ref env_vars) = node.environment_variables
        && !env_vars.is_empty()
    {
        files.extend(build_ignition_user_env(
            env_vars,
            guest_account::guest_username(guest_account),
        ));
    }

    let mut users = vec![ignition_user];
    if let Some(account) = guest_account {
        users.push(guest_account::ignition_user(account));
        files.push(guest_account::sudoers_file(account));
    }
    let mut ignition_config = template::IgnitionConfig::new(
        users,
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

    util::create_dir(&dir)?;
    util::create_file(&src_ztp_file, flatcar_config)?;

    // Copy blank disk for container data
    let src_data_disk =
        format!("{images_dir}/{SHERPA_BLANK_DISK_DIR}/{SHERPA_BLANK_DISK_EXT4_500MB}");
    let dst_disk = format!("{dir}/{node_name_with_lab}-{CONTAINER_DISK_NAME}");

    util::copy_file(&src_data_disk, &dst_disk)?;

    let disk_files = write_binary_files(&dir, node.binary_files.as_deref().unwrap_or_default())?;
    if !disk_files.is_empty() {
        let disk_files: Vec<&str> = disk_files.iter().map(String::as_str).collect();
        util::copy_to_ext4_image(&disk_files, &dst_disk, "/")?;
    }

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
    skip(node, node_image, parts, qemu_commands),
    fields(node_name = %node.name)
)]
pub fn build_domain_template(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_id: &str,
    qemu_bin: &str,
    parts: DomainParts,
    qemu_commands: Vec<data::QemuCommand>,
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
    let vnc_port = resources
        .vnc_port
        .with_context(|| format!("Node '{}' has no VNC port reserved", node.name))?;
    let tuning = node.performance_tuning.as_ref();
    let cpu_model = match tuning.and_then(|tuning| tuning.cpu_mode) {
        Some(mode) => data::CpuModels::from(mode),
        None => node_image.compute.cpu_model.clone(),
    };
    let management_interface_type =
        management_interface_type_for_model(node_image.model, &node_image.interfaces.kind);

    Ok(template::DomainTemplate {
        qemu_bin: qemu_bin.to_string(),
        name: node_name_with_lab,
        memory,
        cpu_architecture: node_image.compute.cpu_architecture.clone(),
        cpu_model,
        machine_type: node_image.compute.machine_type.clone(),
        cpu_count,
        vmx_enabled: node_image.compute.vmx_enabled,
        bios: node_image.compute.bios.clone(),
        disks: parts.disks,
        interfaces: parts.interfaces,
        interface_type: node_image.interfaces.kind.clone(),
        tuning: template::DomainTuning::from_tuning(tuning, &management_interface_type),
        management_interface_type,
        reserved_interface_type: reserved_interface_type_for_model(
            node_image.model,
            &node_image.interfaces.kind,
        ),
        loopback_ipv4: parts.loopback_ipv4,
        telnet_port: TELNET_PORT,
        qemu_commands,
        lab_id: lab_id.to_string(),
        management_network: parts.management_network,
        isolated_network: parts.isolated_network,
        reserved_network: parts.reserved_network,
        is_windows: matches!(
            node_image.model,
            data::NodeModel::WindowsServer | data::NodeModel::DevboxWindows
        ),
        cpu_features: cpu_features_for_model(node_image.model),
        video_model: video_model.to_owned(),
        video_vram,
        vnc_port,
        watchdog: node.watchdog,
    })
}

/// Return the management interface type for a node model.
/// Some platforms (e.g. IOS-XRv9000) require e1000 for the management NIC
/// while using virtio for data interfaces.
fn management_interface_type_for_model(
    model: data::NodeModel,
    default: &data::InterfaceType,
) -> data::InterfaceType {
    match model {
        data::NodeModel::CiscoIosxrv9000 => data::InterfaceType::E1000,
        _ => default.clone(),
    }
}

/// Return the reserved interface type for a node model.
/// Some platforms (e.g. IOS-XRv9000) use e1000 for the reserved system NICs
/// per Cisco documentation.
fn reserved_interface_type_for_model(
    model: data::NodeModel,
    default: &data::InterfaceType,
) -> data::InterfaceType {
    match model {
        data::NodeModel::CiscoIosxrv9000 => data::InterfaceType::E1000,
        _ => default.clone(),
    }
}

/// Return CPU feature overrides for specific node models.
fn cpu_features_for_model(model: data::NodeModel) -> Vec<data::CpuFeature> {
    match model {
        data::NodeModel::JuniperVsrxv3 => {
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

/// Clone a list of disks in parallel using libvirt.
#[instrument(skip(qemu_conn, clone_disks, progress, image_leases), fields(%lab_id), level = "debug")]
pub async fn clone_node_disks(
    qemu_conn: Arc<libvirt::QemuConnection>,
    clone_disks: Vec<data::CloneDisk>,
    lab_id: &str,
    progress: &ProgressSender,
    image_leases: Vec<ImageLease>,
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
    skip(docker, spec, progress),
    fields(container_name = %spec.name, image = %spec.image),
    level = "debug"
)]
pub async fn start_container_node(
    docker: &Arc<bollard::Docker>,
    spec: ContainerSpec,
    model: data::NodeModel,
    progress: &ProgressSender,
) -> Result<bool> {
    let container_name = spec.name.clone();
    let is_running = container::run_container(docker, spec).await?;

    if !is_running {
        return Ok(false);
    }

    // SR Linux: flush default namespace IP to avoid DUP pings
    if model == data::NodeModel::NokiaSrlinux {
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
/// must answer (`check_node_ssh_ready`), and its host key is returned; any other port, such
/// as the Talos API, needs a TCP connection.
pub async fn check_vm_ready(ip: &str, port: u16) -> Result<VmReadiness> {
    if port == SSH_PORT {
        match check_node_ssh_ready(ip, port).await? {
            Some(host_key) => Ok(VmReadiness::Ready {
                host_key: Some(host_key),
            }),
            None => Ok(VmReadiness::NotReady),
        }
    } else if check_node_port_ready(ip, port)? {
        Ok(VmReadiness::Ready { host_key: None })
    } else {
        Ok(VmReadiness::NotReady)
    }
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

/// Port that shows a virtual machine is ready: the manifest's `ready_port`, else the model's
/// own port (SSH for most models, the Talos API for Talos Linux).
pub fn vm_ready_port(model: data::NodeModel, ready_port: Option<u16>) -> u16 {
    ready_port.unwrap_or(model.ready_port())
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
    let boot_mode = node_image
        .compute
        .boot_mode
        .clone()
        .unwrap_or(data::UnikernelBootMode::DirectKernel);

    let mac_address = util::random_mac(KVM_OUI);
    let version = node.version.as_deref().unwrap_or(&node_image.version);
    let filename = util::image_filename(&node_image.kind, node_image.compute.boot_mode.as_ref());

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
                disk_size: node.boot_disk_size,
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
#[instrument(skip(node_image, parts), fields(node_name = %node.name), level = "debug")]
pub fn build_unikernel_domain_template(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_id: &str,
    qemu_bin: &str,
    parts: DomainParts,
    kernel_path: Option<String>,
) -> template::UnikernelDomainTemplate {
    let node_name_with_lab = format!("{}-{}", node.name, lab_id);
    let boot_mode = node_image
        .compute
        .boot_mode
        .clone()
        .unwrap_or(data::UnikernelBootMode::DirectKernel);

    // Only the manifest sets a kernel cmdline: Unikraft's pre-built images take their address
    // by DHCP, from the static MAC-to-IP binding on the Sherpa router, and treat the cmdline as
    // the application path.
    let kernel_cmdline = node.kernel_cmdline.clone();

    template::UnikernelDomainTemplate {
        qemu_bin: qemu_bin.to_string(),
        name: node_name_with_lab,
        memory: node.memory.unwrap_or(node_image.compute.memory),
        cpu_architecture: node_image.compute.cpu_architecture.clone(),
        cpu_model: node_image.compute.cpu_model.clone(),
        machine_type: node_image.compute.machine_type.clone(),
        cpu_count: node.cpu_count.unwrap_or(node_image.compute.cpu_count),
        boot_mode,
        kernel_path,
        kernel_cmdline,
        disks: parts.disks,
        interfaces: parts.interfaces,
        interface_type: node_image.interfaces.kind.clone(),
        management_interface_type: node_image.interfaces.kind.clone(),
        reserved_interface_type: node_image.interfaces.kind.clone(),
        loopback_ipv4: parts.loopback_ipv4,
        telnet_port: TELNET_PORT,
        management_network: parts.management_network,
        isolated_network: parts.isolated_network,
        reserved_network: parts.reserved_network,
    }
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
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::mpsc;

    #[test]
    fn systemd_units_carry_the_contents_the_client_read() {
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
    fn binary_files_are_written_into_the_node_folder() {
        let dir = tempdir().expect("tempdir");
        let node_dir = dir.path().to_string_lossy().into_owned();
        let paths = write_binary_files(
            &node_dir,
            &[BinaryFileData {
                filename: "tool".to_owned(),
                content: util::base64_encode_bytes(&[0, 159, 146, 150]),
            }],
        )
        .expect("write");
        assert_eq!(paths, vec![format!("{node_dir}/{BINARY_FILES_DIR}/tool")]);
        assert_eq!(fs::read(&paths[0]).expect("read"), vec![0, 159, 146, 150]);
        assert_eq!(
            write_binary_files(&node_dir, &[]).expect("none"),
            [] as [std::string::String; 0]
        );
    }

    #[test]
    fn disabled_veths_stay_down_unless_the_model_keeps_them_up() {
        assert_eq!(
            netns_veth_state(data::NodeModel::JuniperCevolved, false),
            NetnsVethState::Enabled
        );
        assert_eq!(
            netns_veth_state(data::NodeModel::JuniperCevolved, true),
            NetnsVethState::DisabledUp
        );
        assert_eq!(
            netns_veth_state(data::NodeModel::AristaCeos, true),
            NetnsVethState::Disabled
        );
    }

    #[test]
    fn user_environment_variables_replace_sherpas_of_the_same_key() {
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
    fn keycloak_serves_https_only_with_the_lab_certificate() {
        let env = keycloak_env_vars(Ipv4Addr::new(172, 31, 1, 22));
        for expected in [
            "KC_DB=dev-file",
            "KC_HTTP_ENABLED=false",
            "KC_HOSTNAME=https://172.31.1.22:8443",
            "KC_HTTPS_CERTIFICATE_FILE=/etc/sherpa/certs/node.crt",
            "KC_HTTPS_CERTIFICATE_KEY_FILE=/etc/sherpa/certs/node.key",
            "KC_BOOTSTRAP_ADMIN_USERNAME=sherpa",
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
    fn test_build_env_profile_script_basic() {
        let env_vars = vec!["EDITOR=vim".to_string(), "LANG=en_US.UTF-8".to_string()];
        let script = build_env_profile_script(&env_vars);
        assert_eq!(
            script,
            "#!/bin/sh\nexport EDITOR='vim'\nexport LANG='en_US.UTF-8'\n"
        );
    }

    #[test]
    fn test_build_env_profile_script_with_single_quotes() {
        let env_vars = vec!["MSG=it's a test".to_string()];
        let script = build_env_profile_script(&env_vars);
        assert_eq!(script, "#!/bin/sh\nexport MSG='it'\\''s a test'\n");
    }

    #[test]
    fn test_build_env_profile_script_with_spaces() {
        let env_vars = vec!["GREETING=hello world".to_string()];
        let script = build_env_profile_script(&env_vars);
        assert_eq!(script, "#!/bin/sh\nexport GREETING='hello world'\n");
    }

    #[test]
    fn test_build_env_profile_script_empty() {
        let env_vars: Vec<String> = vec![];
        let script = build_env_profile_script(&env_vars);
        assert_eq!(script, "#!/bin/sh\n");
    }

    #[test]
    fn test_user_env_file_is_private_to_the_user() {
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
        assert!(file.content.contains("export GH_TOKEN='abc123'"));
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

    #[test]
    fn test_user_env_loader_hooks_the_login_file_in_the_passwd_home() {
        let root = tempdir().unwrap();
        let home = root.path().join("usr/home/alice");
        std::fs::create_dir_all(&home).unwrap();
        let (user, passwd) = fake_passwd(root.path(), &home);

        assert!(run_sh(&[user_env_loader_runcmd(&user, &passwd)]));
        let profile = std::fs::read_to_string(home.join(".profile")).unwrap();
        assert!(profile.contains(r#". "$HOME/.sherpa_env""#));

        std::fs::write(home.join(".bash_profile"), "").unwrap();
        assert!(run_sh(&[user_env_loader_runcmd(&user, &passwd)]));
        let bash_profile = std::fs::read_to_string(home.join(".bash_profile")).unwrap();
        assert!(bash_profile.contains(r#". "$HOME/.sherpa_env""#));
    }

    #[test]
    fn test_user_env_loader_does_nothing_for_an_unknown_user() {
        let root = tempdir().unwrap();
        let (_, passwd) = fake_passwd(root.path(), root.path());
        let runcmd = user_env_loader_runcmd("nosuchuser", &passwd);
        assert!(runcmd.contains(r#"[ -n "$h" ] &&"#));
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
    fn test_staged_files_are_written_root_only_to_the_staging_dir() {
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
    fn test_no_staged_files_means_no_writes_or_runcmd() {
        let (writes, runcmd) = stage_cloud_init_files(&[], "/opt/sherpa/staging", "/etc/passwd");
        assert!(writes.is_empty());
        assert_eq!(runcmd, [] as [std::string::String; 0]);
    }

    /// Runs the generated commands under `sh` to check quoting, home lookup, parent directories,
    /// mode, and removal of the staged copies.
    #[test]
    fn test_staged_files_runcmd_moves_files_into_place_and_removes_staging() {
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
    fn test_staged_files_only_give_the_user_directories_inside_their_home() {
        let files = vec![staged("/home/alice/.config/app.toml", "alice", None)];
        let (_, runcmd) = stage_cloud_init_files(&files, "/opt/sherpa/staging", "/etc/passwd");
        assert!(
            runcmd[0].contains(r#"case "$top" in "$h"|"$h"/*) chown -R "$u:$g" "$top";; esac"#)
        );
        assert!(
            runcmd[0].contains(r#"[ -n "$h" ] &&"#),
            "skipped when the user does not exist"
        );
    }

    #[test]
    fn test_ignition_user_env_is_private_and_appended_to_bash_profile() {
        let files = build_ignition_user_env(&["GH_TOKEN=abc123".to_string()], "alice");
        assert_eq!(files.len(), 2);

        let env_file = &files[0];
        assert_eq!(env_file.path, "/home/alice/.sherpa_env");
        assert_eq!(env_file.mode, template::FileMode::new(0o600));
        assert_eq!(env_file.user.as_ref().unwrap().name, "alice");
        assert_eq!(env_file.group.as_ref().unwrap().name, "alice");
        let source = &env_file.contents.as_ref().unwrap().source;
        let encoded = source.strip_prefix("data:;base64,").unwrap();
        assert!(
            util::base64_decode(encoded)
                .unwrap()
                .contains("export GH_TOKEN='abc123'")
        );

        let profile = &files[1];
        assert_eq!(profile.path, "/home/alice/.bash_profile");
        assert!(profile.contents.is_none(), "append keeps the skeleton file");
        assert_eq!(profile.user.as_ref().unwrap().name, "alice");
        let encoded = profile.append[0]
            .source
            .strip_prefix("data:;base64,")
            .unwrap();
        assert!(
            util::base64_decode(encoded)
                .unwrap()
                .contains(r#". "$HOME/.sherpa_env""#)
        );
    }

    #[test]
    fn test_ignition_cert_files_get_0644_and_0600_modes() {
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

    #[test]
    fn test_openbsd_gets_no_ipv6_in_cloud_init_network_config() {
        let address: Ipv6Addr = "fd00::16".parse().unwrap();
        assert_eq!(
            cloud_init_ipv6_address(data::NodeModel::OpenBsd, Some(address)),
            None
        );
        assert_eq!(
            cloud_init_ipv6_address(data::NodeModel::FreeBsd, Some(address)),
            Some(address)
        );
        assert_eq!(
            cloud_init_ipv6_address(data::NodeModel::UbuntuLinux, Some(address)),
            Some(address)
        );
    }

    #[test]
    fn test_openbsd_wheel_gets_passwordless_doas() {
        let file = cloud_init_doas_file(data::NodeModel::OpenBsd).unwrap();
        assert_eq!(file.path, "/etc/doas.conf");
        assert_eq!(file.content, "permit nopass keepenv :wheel\n");
        assert_eq!(file.permissions, "0400");
        // Root-owned files leave the owner out, so BSD uses root:wheel.
        assert_eq!(file.owner, None);
        assert!(cloud_init_doas_file(data::NodeModel::FreeBsd).is_none());
        assert!(cloud_init_doas_file(data::NodeModel::UbuntuLinux).is_none());
    }

    #[test]
    fn test_openbsd_logins_get_no_sudo_rule() {
        assert_eq!(cloud_init_sudo_rule(data::NodeModel::OpenBsd), None);
        for model in [data::NodeModel::FreeBsd, data::NodeModel::UbuntuLinux] {
            assert_eq!(
                cloud_init_sudo_rule(model).as_deref(),
                Some("ALL=(ALL) NOPASSWD:ALL"),
                "{model}"
            );
        }
    }

    #[test]
    fn test_openbsd_cloud_init_does_not_manage_etc_hosts() {
        assert!(!cloud_init_manage_etc_hosts(data::NodeModel::OpenBsd));
        assert!(cloud_init_manage_etc_hosts(data::NodeModel::FreeBsd));
        assert!(cloud_init_manage_etc_hosts(data::NodeModel::UbuntuLinux));
    }

    fn script(filename: &str) -> topology::StartupScript {
        topology::StartupScript {
            filename: filename.to_string(),
            content: "IyEvYmluL3NoCmVjaG8gaGk=".to_string(),
        }
    }

    #[test]
    fn test_cloud_init_scripts_run_through_their_shebang() {
        let (files, runcmd) = build_cloud_init_scripts(
            "node1",
            Some(&[script("boot.sh")]),
            Some(&[script("user.sh")]),
            "alice",
        )
        .unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "/opt/sherpa/startup_scripts/boot.sh");
        assert_eq!(files[0].content, "#!/bin/sh\necho hi");
        assert_eq!(files[0].permissions, "0755");
        assert_eq!(files[1].path, "/opt/sherpa/user_scripts/user.sh");
        assert_eq!(
            runcmd,
            vec![
                r"'/opt/sherpa/startup_scripts/boot.sh'".to_string(),
                r#"chown -R 'alice':"$(id -gn 'alice')" '/opt/sherpa/user_scripts'"#.to_string(),
                r"su - 'alice' -c ''\''/opt/sherpa/user_scripts/user.sh'\'''".to_string(),
            ]
        );
        assert!(runcmd.iter().all(|line| !line.contains("bash")));
    }

    #[test]
    fn test_cloud_init_scripts_without_user_scripts_skip_the_chown() {
        let (files, runcmd) =
            build_cloud_init_scripts("node1", Some(&[script("boot.sh")]), None, "alice").unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            runcmd,
            vec![r"'/opt/sherpa/startup_scripts/boot.sh'".to_string()]
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
    fn test_ignition_text_file_parents_in_home_are_owned_by_the_user() {
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

    #[test]
    fn test_build_windows_env_commands_basic() {
        let env_vars = vec!["EDITOR=vim".to_string()];
        let commands = build_windows_env_commands(&env_vars);
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0],
            r#"powershell -Command "[System.Environment]::SetEnvironmentVariable('EDITOR', 'vim', 'Machine')""#
        );
    }

    #[test]
    fn test_build_windows_env_commands_with_single_quotes() {
        let env_vars = vec!["MSG=it's here".to_string()];
        let commands = build_windows_env_commands(&env_vars);
        assert_eq!(commands.len(), 1);
        assert!(commands[0].contains("'it''s here'"));
    }

    #[test]
    fn test_build_windows_env_commands_empty() {
        let env_vars: Vec<String> = vec![];
        let commands = build_windows_env_commands(&env_vars);
        assert_eq!(commands, [] as [std::string::String; 0]);
    }

    #[test]
    fn test_build_cloud_init_text_files_basic() {
        let text_files = Some(vec![topology::TextFileData {
            content: "aGVsbG8=".to_string(),
            dst: "/etc/app/config.json".to_string(),
            user: "root".to_string(),
            group: "root".to_string(),
            permissions: 644,
        }]);

        let result = build_cloud_init_text_files(text_files.as_deref());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].dst, "/etc/app/config.json");
        assert_eq!(result[0].content, "aGVsbG8=");
        assert_eq!(result[0].permissions, "644");
        assert_eq!(result[0].user, "root");
        assert_eq!(result[0].group, Some("root".to_string()));
        assert_eq!(result[0].encoding, Some("b64".to_string()));
    }

    #[test]
    fn test_build_cloud_init_text_files_none() {
        let result = build_cloud_init_text_files(None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_cloud_init_text_files_multiple() {
        let text_files = Some(vec![
            topology::TextFileData {
                content: "Zmls".to_string(),
                dst: "/etc/file1".to_string(),
                user: "root".to_string(),
                group: "root".to_string(),
                permissions: 644,
            },
            topology::TextFileData {
                content: "c2NyaXB0".to_string(),
                dst: "/opt/file2".to_string(),
                user: "sherpa".to_string(),
                group: "sherpa".to_string(),
                permissions: 755,
            },
        ]);

        let result = build_cloud_init_text_files(text_files.as_deref());
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].dst, "/etc/file1");
        assert_eq!(result[1].dst, "/opt/file2");
        assert_eq!(result[1].permissions, "755");
        assert_eq!(result[1].user, "sherpa");
        assert_eq!(result[1].group, Some("sherpa".to_string()));
    }

    #[test]
    fn test_build_cloudbase_text_files_basic() {
        let text_files = Some(vec![topology::TextFileData {
            content: "aGVsbG8=".to_string(),
            dst: r"C:\app\config.json".to_string(),
            user: "Administrator".to_string(),
            group: "Administrators".to_string(),
            permissions: 644,
        }]);

        let result = build_cloudbase_text_files(text_files.as_deref());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, r"C:\app\config.json");
        assert_eq!(result[0].content, "aGVsbG8=");
        assert_eq!(result[0].permissions, "0644");
    }

    #[test]
    fn test_build_cloudbase_text_files_none() {
        let result = build_cloudbase_text_files(None);
        assert!(result.is_empty());
    }

    fn cloudbase_file(path: &str, permissions: &str) -> template::CloudbaseWriteFile {
        template::CloudbaseWriteFile {
            path: path.to_string(),
            content: "content".to_string(),
            permissions: permissions.to_string(),
        }
    }

    #[test]
    fn test_windows_private_file_acls_lock_down_files_others_cannot_read() {
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
    fn test_windows_private_file_acls_none_for_readable_files() {
        let files = vec![cloudbase_file(r"C:\app\config.json", "0644")];
        assert_eq!(
            build_windows_private_file_acls(&files).unwrap(),
            [] as [std::string::String; 0]
        );
    }

    #[test]
    fn test_windows_private_file_acls_reject_paths_cmd_would_reinterpret() {
        for path in [r#"C:\app\a" & del C:\x & ".txt"#, r"C:\app\%TEMP%.txt"] {
            let files = vec![cloudbase_file(path, "0600")];
            assert!(build_windows_private_file_acls(&files).is_err(), "{path}");
        }
    }

    // =========================================================================
    // Cmdline format function tests
    // =========================================================================

    /// Generate Unikraft kernel cmdline args with network configuration.
    ///
    /// Produces a cmdline string in the format Unikraft's lwIP/uknetdev stack expects:
    /// `netdev.ipv4_addr=<ip> netdev.ipv4_gw_addr=<gw> netdev.ipv4_subnet_mask=<mask>`
    ///
    /// Note: Unikraft does not support IPv6 via kernel cmdline (source has `/* TODO: ip6 */`).
    /// Not used in production (Unikraft pre-built images use DHCP), but kept as a reference
    /// for the expected cmdline format and tested to ensure correctness.
    fn unikraft_cmdline(ipv4: &str, subnet_mask: &str, gateway_v4: &str) -> String {
        format!(
            "netdev.ipv4_addr={ipv4} netdev.ipv4_gw_addr={gateway_v4} \
             netdev.ipv4_subnet_mask={subnet_mask}"
        )
    }

    #[test]
    fn test_unikraft_cmdline_format() {
        let result = unikraft_cmdline("10.0.0.10", "255.255.255.0", "10.0.0.1");
        assert!(result.contains("netdev.ipv4_addr=10.0.0.10"));
        assert!(result.contains("netdev.ipv4_gw_addr=10.0.0.1"));
        assert!(result.contains("netdev.ipv4_subnet_mask=255.255.255.0"));
        // Unikraft does not support IPv6 via cmdline
        assert!(!result.contains("ipv6"));
    }
    #[test]
    fn test_omarchy_cloud_init_user_settings() {
        assert_eq!(
            cloud_init_user_settings(data::NodeModel::OmarchyLinux, &data::OsVariant::Linux),
            ("wheel", "/bin/bash")
        );
        assert_eq!(
            cloud_init_user_settings(data::NodeModel::UbuntuLinux, &data::OsVariant::Linux),
            ("sudo", "/bin/bash")
        );
        assert_eq!(
            cloud_init_user_settings(data::NodeModel::FreeBsd, &data::OsVariant::Bsd),
            ("wheel", "/bin/sh")
        );
    }

    #[test]
    fn test_nixos_cloud_init_uses_wheel_and_the_system_bash() {
        assert_eq!(
            cloud_init_user_settings(data::NodeModel::NixosLinux, &data::OsVariant::Linux),
            ("wheel", NODE_NIXOS_SHELL)
        );
        // NixOS generates /etc/sudoers without /etc/sudoers.d, so wheel grants sudo instead.
        assert!(cloud_init_sudo_rule(data::NodeModel::NixosLinux).is_none());
        assert!(cloud_init_sudo_rule(data::NodeModel::UbuntuLinux).is_some());
    }

    #[test]
    fn test_nixos_cloud_init_leaves_ssh_password_auth_to_the_image() {
        // NixOS generates sshd_config, which cloud-init cannot edit.
        assert_eq!(cloud_init_ssh_pwauth(data::NodeModel::NixosLinux), None);
        assert_eq!(
            cloud_init_ssh_pwauth(data::NodeModel::UbuntuLinux),
            Some(true)
        );
    }

    #[test]
    fn test_omarchy_uses_standard_vm_domain() {
        let config = data::NodeConfig::omarchy_linux();
        let node = topology::NodeExpanded {
            name: "desktop".to_owned(),
            model: config.model,
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
                loopback_ipv4: "127.0.0.1".to_owned(),
                management_network: "mgmt".to_owned(),
                isolated_network: String::new(),
                reserved_network: String::new(),
            },
            vec![],
            &data::NodeResources {
                cpu_count: Some(2),
                memory: Some(4096),
                vnc_port: Some(5900),
                ..data::NodeResources::default()
            },
        )
        .unwrap();
        let xml = domain.render().unwrap();
        assert_eq!(domain.video_model, "virtio");
        assert!(domain.video_vram.is_none());
        assert!(xml.contains("<model type='virtio' heads='1'/>"));
        assert!(xml.contains("OVMF_CODE_4M.fd"));
        assert!(xml.find("<boot dev='cdrom'/>").unwrap() < xml.find("<boot dev='hd'/>").unwrap());
        assert_eq!(config.ztp.mode, data::Ztp::CloudInit);
        assert!(config.disks.cdrom.is_none());
    }

    /// The sherpa login every Ignition test uses, with a fixture key.
    fn ignition_test_user() -> data::User {
        data::User {
            username: SHERPA_USERNAME.to_owned(),
            password: None,
            ssh_public_key: data::SshPublicKey {
                algorithm: data::SshKeyAlgorithms::SshEd25519,
                key: "legacy-key-fixture".to_owned(),
                comment: None,
            },
            sudo: true,
        }
    }

    /// The rule in an Ignition sudoers file's base64 data URL.
    fn ignition_file_text(file: &serde_json::Value) -> String {
        util::base64_decode(
            file["contents"]["source"]
                .as_str()
                .unwrap()
                .strip_prefix("data:;base64,")
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn flatcar_sudoers_matches_existing_ignition_provisioning() {
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
            model: data::NodeModel::FlatcarLinux,
            ..Default::default()
        };
        let image = data::NodeConfig::get_model(data::NodeModel::FlatcarLinux);
        let user = ignition_test_user();
        let account = GuestAccount {
            username: "alice".to_owned(),
            ssh_keys: vec!["ssh-ed25519 personal-key-fixture".to_owned()],
        };
        let network = nocloud_test_network();
        let (tx, _rx) = mpsc::unbounded_channel();
        let progress = ProgressSender::new(tx);
        let dns = util::default_dns(&network.v4.prefix).unwrap();
        for personal in [None, Some(&account)] {
            let ztp = NodeZtp {
                lab_dir: root,
                tftp_dir: root,
                images_dir: root,
                sherpa_user: &user,
                dns: &dns,
                mgmt_net: &network,
                progress: &progress,
                node_ipv4_address: "192.0.2.10".parse().unwrap(),
                mac_address: "52:54:00:00:00:01",
                node_name_with_lab: "vm-test",
                cert_paths: None,
                guest_account: personal,
            };
            let (config_path, _, _, _) = generate_ignition_ztp(&node, &image, &ztp).unwrap();
            let json: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(config_path).unwrap()).unwrap();
            let files = json["storage"]["files"].as_array().unwrap();
            let legacy = files
                .iter()
                .find(|file| file["path"] == format!("/etc/sudoers.d/{SHERPA_USERNAME}"))
                .unwrap();
            assert_eq!(legacy["mode"], 0o440);
            assert_eq!(
                ignition_file_text(legacy),
                format!("{SHERPA_USERNAME} ALL=(ALL) NOPASSWD: ALL")
            );
            assert_eq!(
                json["passwd"]["users"][0]["groups"],
                serde_json::json!(["wheel", "docker"])
            );
            let personal_file = files
                .iter()
                .find(|file| file["path"] == "/etc/sudoers.d/alice");
            if personal.is_some() {
                let personal_file = personal_file.unwrap();
                assert_eq!(personal_file["mode"], legacy["mode"]);
                assert_eq!(personal_file.get("user"), legacy.get("user"));
                assert_eq!(personal_file.get("group"), legacy.get("group"));
                assert_eq!(
                    ignition_file_text(personal_file),
                    "alice ALL=(ALL) NOPASSWD: ALL\n"
                );
                assert_eq!(json["passwd"]["users"][1]["name"], "alice");
                assert!(json["passwd"]["users"][1].get("passwordHash").is_none());
            } else {
                assert!(personal_file.is_none());
                assert_eq!(json["passwd"]["users"].as_array().unwrap().len(), 1);
            }
        }
    }

    #[test]
    fn cloud_init_gives_the_sherpa_account_the_labs_key() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let user = ignition_test_user();
        let network = nocloud_test_network();
        let (tx, _rx) = mpsc::unbounded_channel();
        let progress = ProgressSender::new(tx);
        let dns = util::default_dns(&network.v4.prefix).unwrap();
        let ztp = NodeZtp {
            lab_dir: root,
            tftp_dir: root,
            images_dir: root,
            sherpa_user: &user,
            dns: &dns,
            mgmt_net: &network,
            progress: &progress,
            node_ipv4_address: "192.0.2.10".parse().unwrap(),
            mac_address: "52:54:00:00:00:01",
            node_name_with_lab: "vm-test",
            cert_paths: None,
            guest_account: None,
        };
        for model in [
            data::NodeModel::UbuntuLinux,
            data::NodeModel::AlpineLinux,
            data::NodeModel::WindowsServer,
        ] {
            let node = topology::NodeExpanded {
                name: model.to_string(),
                model,
                ..Default::default()
            };
            let image = data::NodeConfig::get_model(model);
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
            assert!(
                user_data.contains("ssh-ed25519 legacy-key-fixture"),
                "{model}: {user_data}"
            );
        }
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
    fn nocloud_files_carry_the_node_config_and_a_static_management_address() {
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
        // It holds the node's secrets, so only sherpad may read it.
        let mode = fs::metadata(dir.join(CLOUD_INIT_USER_DATA))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
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
    fn nocloud_files_without_a_config_leave_the_node_in_maintenance_mode() {
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
    fn vm_ready_port_prefers_the_manifest_then_the_model() {
        assert_eq!(
            vm_ready_port(data::NodeModel::TalosLinux, None),
            TALOS_API_PORT
        );
        assert_eq!(vm_ready_port(data::NodeModel::UbuntuLinux, None), SSH_PORT);
        assert_eq!(
            vm_ready_port(data::NodeModel::UbuntuLinux, Some(8443)),
            8443
        );
    }

    #[test]
    fn test_check_node_port_ready_invalid_address_returns_error() {
        let result = check_node_port_ready("not_an_ip", 8080);
        assert!(result.is_err());
    }

    #[test]
    fn test_check_node_port_ready_closed_port_returns_false() {
        // Port 1 is almost certainly not listening
        let result = check_node_port_ready("127.0.0.1", 1);
        assert!(result.is_ok());
        assert!(!result.unwrap());
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

    #[tokio::test]
    async fn test_check_node_ssh_ready_false_when_the_port_is_not_ssh() {
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
    async fn test_check_node_ssh_ready_false_when_closed() {
        assert!(
            check_node_ssh_ready("127.0.0.1", 1)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_check_vm_ready_uses_tcp_for_other_ports() {
        let (port, task) = listener_replying(b"not ssh\r\n").await;
        assert_eq!(
            check_vm_ready("127.0.0.1", port).await.unwrap(),
            VmReadiness::Ready { host_key: None }
        );
        task.abort();
    }

    #[test]
    fn test_ceos_startup_commands_copy_the_ztp_config_into_flash_once() {
        let commands = ceos_startup_commands();
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0], "sh");
        assert_eq!(commands[1], "-c");
        // EOS saves by replacing startup-config, which a bind-mounted file cannot be, so the
        // ZTP config is copied into flash. A config already saved there is kept.
        assert_eq!(
            commands[2],
            "[ -f /mnt/flash/startup-config ] || cp /mnt/sherpa-ztp/startup-config \
             /mnt/flash/startup-config; exec /sbin/init systemd.setenv=INTFTYPE=eth \
             systemd.setenv=ETBA=1 systemd.setenv=SKIP_ZEROTOUCH_BARRIER_IN_SYSDBINIT=1 \
             systemd.setenv=CEOS=1 systemd.setenv=EOS_PLATFORM=ceoslab \
             systemd.setenv=container=docker systemd.setenv=MAPETH0=1 \
             systemd.setenv=MGMT_INTF=eth0"
        );
    }

    /// A lab folder and the accounts and network its nodes are configured with.
    struct ZtpLab {
        dir: tempfile::TempDir,
        user: data::User,
        network: data::SherpaNetwork,
        dns: data::Dns,
        progress: ProgressSender,
        messages: mpsc::UnboundedReceiver<axum::extract::ws::Message>,
    }

    impl ZtpLab {
        fn new() -> Self {
            let network = nocloud_test_network();
            let (tx, messages) = mpsc::unbounded_channel();
            Self {
                dir: tempdir().unwrap(),
                user: ignition_test_user(),
                dns: util::default_dns(&network.v4.prefix).unwrap(),
                network,
                progress: ProgressSender::new(tx),
                messages,
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
                sherpa_user: &self.user,
                dns: &self.dns,
                mgmt_net: &self.network,
                progress: &self.progress,
                owner_account: None,
            }
        }

        /// Generate the container node's setup with `ztp` as its manifest setting.
        fn container(
            &self,
            node: &mut topology::NodeExpanded,
            ztp: Option<data::Ztp>,
        ) -> Result<ContainerZtpResult> {
            node.ztp = ztp;
            generate_container_ztp(
                node,
                &data::NodeConfig::get_model(node.model),
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
            model,
            ..Default::default()
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    const MOUNTED_CONFIG_MODELS: [data::NodeModel; 4] = [
        data::NodeModel::AristaCeos,
        data::NodeModel::NokiaSrlinux,
        data::NodeModel::JuniperCevolved,
        data::NodeModel::FrrLinux,
    ];

    #[test]
    fn a_vm_with_ztp_off_gets_no_media_and_a_disabled_record() {
        let mut lab = ZtpLab::new();
        let mut node = named_node("vm01", data::NodeModel::UbuntuLinux);
        node.ztp = Some(data::Ztp::Disabled);
        let image = data::NodeConfig::get_model(data::NodeModel::UbuntuLinux);

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
        assert!(result.qemu_commands.is_empty());
        assert!(lab.files().is_empty(), "{:?}", lab.files());
        let mut messages = Vec::new();
        while let Ok(message) = lab.messages.try_recv() {
            messages.push(format!("{message:?}"));
        }
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.contains("ZTP is off for node: vm01"))
                .count(),
            1,
            "{messages:?}"
        );
    }

    #[test]
    fn a_vm_follows_its_images_ztp_when_the_manifest_sets_none() {
        let lab = ZtpLab::new();
        let mut node = named_node("talos01", data::NodeModel::TalosLinux);
        let image = data::NodeConfig::get_model(data::NodeModel::TalosLinux);

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
        assert!(lab.files().contains(&format!("talos01/{ZTP_ISO}")));
    }

    #[test]
    fn volume_on_a_vm_is_an_error_not_a_skipped_step() {
        let lab = ZtpLab::new();
        let mut node = named_node("vm01", data::NodeModel::UbuntuLinux);
        node.ztp = Some(data::Ztp::Volume);
        let image = data::NodeConfig::get_model(data::NodeModel::UbuntuLinux);

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
    fn ceos_with_volume_ztp_mounts_its_config_and_copies_it_into_flash() {
        let lab = ZtpLab::new();
        let mut node = named_node("ceos01", data::NodeModel::AristaCeos);

        let result = lab.container(&mut node, None).unwrap();

        let config = format!("{}/ceos01/ceos01.conf", lab.root());
        assert_eq!(result.ztp_record.ztp, data::Ztp::Volume);
        assert_eq!(lab.files(), ["ceos01/ceos01.conf"]);
        assert_eq!(
            result.volumes,
            [format!("{config}:{ARISTA_CEOS_ZTP_VOLUME_MOUNT}:ro")]
        );
        assert_eq!(result.commands, ceos_startup_commands());
        assert!(
            fs::read_to_string(config)
                .unwrap()
                .contains(SHERPA_USERNAME)
        );
    }

    #[test]
    fn ceos_with_ztp_off_starts_with_the_plain_init_command() {
        let lab = ZtpLab::new();
        let mut on = named_node("ceos01", data::NodeModel::AristaCeos);
        let with_ztp = lab.container(&mut on, None).unwrap();
        let lab = ZtpLab::new();
        let mut node = named_node("ceos01", data::NodeModel::AristaCeos);

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
    fn srlinux_mounts_its_config_only_with_volume_ztp() {
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
    fn cevolved_mounts_its_config_directory_only_with_volume_ztp() {
        let lab = ZtpLab::new();
        let mut node = named_node("ptx01", data::NodeModel::JuniperCevolved);
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
        let mut node = named_node("ptx01", data::NodeModel::JuniperCevolved);
        let off = lab.container(&mut node, Some(data::Ztp::Disabled)).unwrap();
        assert!(lab.files().is_empty(), "{:?}", lab.files());
        assert!(off.volumes.is_empty(), "{:?}", off.volumes);
        // The entrypoint still gets the environment it starts the EVO VM from.
        assert_eq!(off.image, with_ztp.image);
        assert_eq!(off.env_vars, with_ztp.env_vars);
        assert!(
            off.env_vars
                .iter()
                .any(|var| var.starts_with("CPTX_NUM_IF="))
        );
        assert!(off.privileged);
    }

    #[test]
    fn frr_gets_its_daemons_config_and_startup_script_only_with_volume_ztp() {
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
    fn a_container_refuses_a_method_that_is_not_volume() {
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
                let mut node = named_node("node1", model);
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
    fn volume_on_a_container_without_a_config_generator_is_an_error() {
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
    fn application_containers_keep_their_start_settings() {
        for (model, commands) in [
            (data::NodeModel::GitlabCe, CONTAINER_GITLAB_CE_COMMANDS),
            (data::NodeModel::ForgejoForge, CONTAINER_FORGEJO_COMMANDS),
            (data::NodeModel::SurrealDb, CONTAINER_SURREAL_DB_COMMANDS),
            (data::NodeModel::MongoDb, CONTAINER_MONGO_DB_COMMANDS),
            (data::NodeModel::NatsBus, CONTAINER_NATS_COMMANDS),
        ] {
            let lab = ZtpLab::new();
            let mut node = named_node("app01", model);
            let result = lab.container(&mut node, None).unwrap();
            assert_eq!(result.ztp_record.ztp, data::Ztp::Disabled, "{model}");
            assert_eq!(result.commands, strings(commands), "{model}");
            assert!(result.volumes.is_empty(), "{model}");
            assert!(!result.privileged, "{model}");
            assert!(lab.files().is_empty(), "{model}");
        }
        let lab = ZtpLab::new();
        let mut node = named_node("git01", data::NodeModel::GitlabCe);
        let gitlab = lab.container(&mut node, None).unwrap();
        assert_eq!(
            gitlab.shm_size,
            Some(CONTAINER_GITLAB_CE_SHM_SIZE * 1024 * 1024)
        );
    }

    #[test]
    fn containers_with_a_built_in_repo_run_the_image_their_node_resolved() {
        let certs = NodeCertPaths {
            ca_cert: "/certs/ca.crt".to_owned(),
            node_cert: "/certs/node.crt".to_owned(),
            node_key: "/certs/node.key".to_owned(),
        };
        let models: Vec<data::NodeModel> = data::NodeModel::to_vec()
            .into_iter()
            .filter(|model| {
                let config = data::NodeConfig::get_model(*model);
                config.kind == data::NodeKind::Container && config.repo.is_some()
            })
            .collect();
        assert_eq!(models.len(), 11, "{models:?}");
        for model in models {
            let lab = ZtpLab::new();
            let mut node = named_node("node1", model);
            node.image = Some("registry.example:5000/mirror/image".to_owned());

            let result = generate_container_ztp(
                &mut node,
                &data::NodeConfig::get_model(model),
                &lab.lab(),
                "192.0.2.10".parse().unwrap(),
                Some(&certs),
            )
            .unwrap();

            assert_eq!(
                result.image, "registry.example:5000/mirror/image",
                "{model}"
            );
            assert_eq!(
                node.image.as_deref(),
                Some("registry.example:5000/mirror/image"),
                "{model}"
            );
        }
    }

    #[test]
    fn containers_without_a_model_setup_keep_the_manifests_settings() {
        for model in [
            data::NodeModel::MysqlDb,
            data::NodeModel::PostgresqlDb,
            data::NodeModel::GenericContainer,
        ] {
            let lab = ZtpLab::new();
            let mut node = named_node("app01", model);
            node.image = Some("example/app".to_owned());
            node.commands = Some(strings(&["serve"]));
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
    fn vault_keeps_its_own_config_as_a_start_setting_with_ztp_off() {
        let lab = ZtpLab::new();
        let mut node = named_node("vault01", data::NodeModel::HashicorpVault);
        node.ztp_config = Some("ui = true".to_owned());

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
            &data::NodeConfig::get_model(data::NodeModel::KeycloakServer),
            &lab.lab(),
            "192.0.2.10".parse().unwrap(),
            Some(&certs),
        )
        .unwrap();

        assert_eq!(result.ztp_record.ztp, data::Ztp::Disabled);
        assert_eq!(result.commands, strings(CONTAINER_KEYCLOAK_COMMANDS));
        assert_eq!(result.user, Some(CONTAINER_KEYCLOAK_USER.to_owned()));
        assert_eq!(result.volumes.len(), 3, "the certificate mounts");
        assert!(lab.files().is_empty(), "{:?}", lab.files());
    }

    fn manifest_volume() -> Vec<topology::VolumeMount> {
        vec![topology::VolumeMount {
            src: "/srv/own.conf".to_owned(),
            dst: "/etc/own.conf".to_owned(),
        }]
    }

    #[test]
    fn manifest_volumes_reach_a_mounted_config_model_only_with_ztp_off() {
        for model in MOUNTED_CONFIG_MODELS {
            let lab = ZtpLab::new();
            let mut node = named_node("node1", model);
            node.volumes = Some(manifest_volume());
            let off = lab.container(&mut node, Some(data::Ztp::Disabled)).unwrap();
            assert_eq!(off.volumes, ["/srv/own.conf:/etc/own.conf"], "{model}");

            let lab = ZtpLab::new();
            let mut node = named_node("node1", model);
            node.volumes = Some(manifest_volume());
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
            node.volumes = Some(manifest_volume());
            node.ztp_config = Some("ui = true".to_owned());
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
        node.volumes = Some(manifest_volume());
        let result = lab.container(&mut node, None).unwrap();
        assert_eq!(result.volumes, ["/srv/own.conf:/etc/own.conf"]);
    }
}
