// Shared per-node operations extracted from up.rs for reuse by redeploy.rs

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

use crate::services::guest_account::{self, GuestAccount};
use crate::services::image_lifecycle::ImageLease;
use crate::services::progress::ProgressSender;

use shared::data;
use shared::data::StatusKind;
use shared::konst::{
    ARISTA_ABOOT_DIR, ARISTA_CEOS_ZTP_VOLUME_MOUNT, CISCO_ASAV_ZTP_CONFIG, CISCO_FTDV_ZTP_CONFIG,
    CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_ZTP_CONFIG, CISCO_IOSXR_ZTP_CONFIG, CISCO_ISE_ZTP_CONFIG,
    CISCO_NXOS_ZTP_CONFIG, CLOUD_INIT_META_DATA, CLOUD_INIT_NETWORK_CONFIG, CLOUD_INIT_USER_DATA,
    CONTAINER_ARISTA_CEOS_COMMANDS, CONTAINER_ARISTA_CEOS_ENV_VARS, CONTAINER_ARISTA_CEOS_REPO,
    CONTAINER_DISK_NAME, CONTAINER_FORGEJO_COMMANDS, CONTAINER_FORGEJO_REPO,
    CONTAINER_FRR_ENV_VARS, CONTAINER_FRR_REPO, CONTAINER_GITLAB_CE_COMMANDS,
    CONTAINER_GITLAB_CE_REPO, CONTAINER_GITLAB_CE_SHM_SIZE, CONTAINER_HASHICORP_VAULT_CAPABILITIES,
    CONTAINER_HASHICORP_VAULT_COMMANDS, CONTAINER_HASHICORP_VAULT_ENV_VARS,
    CONTAINER_HASHICORP_VAULT_REPO, CONTAINER_MONGO_DB_COMMANDS, CONTAINER_MONGO_DB_REPO,
    CONTAINER_NATS_COMMANDS, CONTAINER_NATS_REPO, CONTAINER_NOKIA_SRLINUX_COMMANDS,
    CONTAINER_NOKIA_SRLINUX_ENV_VARS, CONTAINER_NOKIA_SRLINUX_REPO, CONTAINER_NOKIA_SRLINUX_USER,
    CONTAINER_SURREAL_DB_COMMANDS, CONTAINER_SURREAL_DB_REPO, CUMULUS_ZTP, FRR_ZTP_CONFIG_MOUNT,
    FRR_ZTP_DAEMONS_MOUNT, FRR_ZTP_STARTUP_MOUNT, JUNIPER_ZTP_CONFIG, JUNIPER_ZTP_CONFIG_TGZ,
    KVM_OUI, MIKROTIK_CHR_ZTP_CONFIG, NODE_CERTS_DIR, NODE_CERTS_DIR_WINDOWS,
    NODE_CLOUD_INIT_SUDO_RULE, NODE_CONFIGS_DIR, NODE_DOAS_CONF_FILE, NODE_PASSWD_FILE,
    NODE_STAGING_DIR, NODE_STARTUP_SCRIPTS_DIR, NODE_USER_ENV_FILE, NODE_USER_SCRIPTS_DIR,
    NOKIA_SRLINUX_ZTP_VOLUME_MOUNT, PALOALTO_BOOTSTRAP_CONFIG, PALOALTO_ZTP_CONFIG,
    SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_EXT4_500MB, SHERPA_BLANK_DISK_FAT32,
    SHERPA_BLANK_DISK_IOSV, SHERPA_BLANK_DISK_ISE, SHERPA_BLANK_DISK_JUNOS, SHERPA_DOMAIN_NAME,
    SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX, SHERPA_ISOLATED_NETWORK_NAME, SHERPA_PASSWORD,
    SHERPA_PASSWORD_HASH, SHERPA_PASSWORD_HASH_SHA256, SHERPA_RESERVED_NETWORK_BRIDGE_PREFIX,
    SHERPA_RESERVED_NETWORK_NAME, SHERPA_SSH_PUBLIC_KEY_PATH, SHERPA_STORAGE_POOL,
    SHERPA_STORAGE_POOL_PATH, SHERPA_USERNAME, SSH_PORT, SSH_READY_HANDSHAKE_TIMEOUT,
    TALOS_API_PORT, TELNET_PORT, VAULT_ZTP_CONFIG_MOUNT, WINDOWS_ADMINISTRATORS_SID,
    WINDOWS_SYSTEM_SID, ZTP_DIR, ZTP_ISO, ZTP_JSON,
};
use shared::util;
use virt::error::ErrorNumber;
use virt::sys::VIR_DOMAIN_UNDEFINE_NVRAM;

// ============================================================================
// Result structs
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

/// Paths to TLS certificate files for a node
pub struct NodeCertPaths {
    pub ca_cert: String,
    pub node_cert: String,
    pub node_key: String,
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
// Helper functions (moved from up.rs)
// ============================================================================

pub fn node_isolated_network_data(
    node_name: &str,
    node_index: u16,
    lab_id: &str,
) -> data::LabIsolatedNetwork {
    data::LabIsolatedNetwork {
        network_name: format!("{}-{}-{}", SHERPA_ISOLATED_NETWORK_NAME, node_name, lab_id),
        bridge_name: format!(
            "{}{}-{}",
            SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX, node_index, lab_id
        ),
    }
}

pub fn node_reserved_network_data(
    node_name: &str,
    node_index: u16,
    lab_id: &str,
) -> data::LabReservedNetwork {
    data::LabReservedNetwork {
        network_name: format!("{}-{}-{}", SHERPA_RESERVED_NETWORK_NAME, node_name, lab_id),
        bridge_name: format!(
            "{}{}-{}",
            SHERPA_RESERVED_NETWORK_BRIDGE_PREFIX, node_index, lab_id
        ),
    }
}

/// Write the NoCloud seed files for a node into `dir`.
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
        meta_data.to_string()?,
    )?;

    let network_config = template::CloudInitNetwork::ztp_interface(
        node_ipv4_address,
        mac_address.to_owned(),
        mgmt_net.v4.clone(),
        node_ipv6_address,
        mgmt_net.v6.as_ref(),
    );
    util::create_file(
        &format!("{dir}/{CLOUD_INIT_NETWORK_CONFIG}"),
        network_config.to_string()?,
    )?;

    let user_data_path = format!("{dir}/{CLOUD_INIT_USER_DATA}");
    match user_data {
        Some(content) => util::create_file(&user_data_path, content.to_owned())?,
        None => match fs::remove_file(&user_data_path) {
            Ok(()) => tracing::debug!(path = %user_data_path, "Removed stale user-data"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %user_data_path, "No user-data to remove")
            }
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to remove {user_data_path}"));
            }
        },
    }
    Ok(())
}

/// Build the NoCloud `cidata` ISO for a node.
#[tracing::instrument(
    level = "debug",
    skip(node, mgmt_net, user_data, progress),
    fields(node_name = %node.name)
)]
fn generate_nocloud_ztp(
    node: &topology::NodeExpanded,
    lab_dir: &str,
    mgmt_net: &data::SherpaNetwork,
    node_ipv4_address: Ipv4Addr,
    mac_address: &str,
    user_data: Option<&str>,
    progress: &ProgressSender,
) -> Result<()> {
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
    util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
    Ok(())
}

/// Map a node model to its ZTP config filename for CDROM-based ZTP.
pub fn ztp_config_filename(model: &data::NodeModel) -> Result<String> {
    let name = match model {
        data::NodeModel::CiscoCsr1000v
        | data::NodeModel::CiscoCat8000v
        | data::NodeModel::CiscoCat9000v => CISCO_IOSXE_ZTP_CONFIG.replace("-", "_"),
        data::NodeModel::CiscoAsav => CISCO_ASAV_ZTP_CONFIG.to_string(),
        data::NodeModel::CiscoNexus9300v => CISCO_NXOS_ZTP_CONFIG.to_string(),
        data::NodeModel::CiscoIosxrv9000 => CISCO_IOSXR_ZTP_CONFIG.to_string(),
        data::NodeModel::CiscoFtdv => CISCO_FTDV_ZTP_CONFIG.to_string(),
        data::NodeModel::JuniperVsrxv3
        | data::NodeModel::JuniperVrouter
        | data::NodeModel::JuniperVswitch => JUNIPER_ZTP_CONFIG.to_string(),
        data::NodeModel::PaloaltoPanos => PALOALTO_ZTP_CONFIG.to_string(),
        _ => bail!("No ZTP config filename mapping for model: {}", model),
    };
    Ok(name)
}

/// If the node has a custom ZTP config, log and send a progress message, then return the content.
pub fn take_custom_ztp_config(
    node: &mut topology::NodeExpanded,
    progress: &ProgressSender,
) -> Result<Option<String>> {
    match node.ztp_config.take() {
        Some(config) => {
            tracing::info!(node_name = %node.name, "Using custom ZTP config for node");
            let _ = progress.send_status(
                format!("Using custom ZTP config for node: {}", node.name),
                StatusKind::Info,
            );
            Ok(Some(config))
        }
        None => Ok(None),
    }
}

pub fn get_node_data(node_name: &str, data: &[data::NodeSetupData]) -> Result<data::NodeSetupData> {
    Ok(data
        .iter()
        .find(|x| x.name == node_name)
        .ok_or_else(|| anyhow!("Node setup data not found for node: {}", node_name))?
        .clone())
}

// ============================================================================
// Container ZTP Generation
// ============================================================================

/// Return Linux capabilities required by a given container node model.
pub fn model_capabilities(model: &data::NodeModel) -> Vec<String> {
    match model {
        data::NodeModel::HashicorpVault => CONTAINER_HASHICORP_VAULT_CAPABILITIES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        _ => Vec::new(),
    }
}

/// Generate ZTP configuration for a container node.
/// Returns the image, env_vars, volumes, commands, privileged flag, user, and ZTP record.
#[allow(clippy::too_many_arguments)]
pub fn generate_container_ztp(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_dir: &str,
    sherpa_user: &data::User,
    dns: &data::Dns,
    mgmt_net: &data::SherpaNetwork,
    node_ipv4_address: Ipv4Addr,
    progress: &ProgressSender,
    cert_paths: Option<&NodeCertPaths>,
) -> Result<ContainerZtpResult> {
    let dir = format!("{}/{}", lab_dir, node.name);

    let custom_ztp = take_custom_ztp_config(node, progress)?;

    let mut image = node.image.clone().unwrap_or_default();
    let mut env_vars: Vec<String> = node.environment_variables.clone().unwrap_or_default();
    let mut volumes: Vec<String> = Vec::new();
    let mut commands: Vec<String> = node.commands.clone().unwrap_or_default();
    let mut capabilities: Vec<String> = Vec::new();
    let mut privileged = node.privileged.unwrap_or(false);
    let mut shm_size = node.shm_size;
    let mut user = node.user.clone();

    match node.model {
        data::NodeModel::AristaCeos => {
            let rendered_template = match &custom_ztp {
                Some(config) => config.clone(),
                None => {
                    let arista_template = template::AristaCeosZtpTemplate {
                        hostname: node.name.clone(),
                        user: sherpa_user.clone(),
                        dns: dns.clone(),
                        mgmt_ipv4_address: Some(node_ipv4_address),
                        mgmt_ipv4: mgmt_net.v4.clone(),
                        mgmt_ipv6_address: node.ipv6_address,
                        mgmt_ipv6: mgmt_net.v6.clone(),
                    };
                    arista_template.render()?
                }
            };
            let ztp_config = format!("{dir}/{}.conf", node.name);
            let ztp_volume = topology::VolumeMount {
                src: ztp_config.clone(),
                dst: ARISTA_CEOS_ZTP_VOLUME_MOUNT.to_string(),
            };
            util::create_dir(&dir)?;
            util::create_file(&ztp_config, rendered_template)?;

            image = CONTAINER_ARISTA_CEOS_REPO.to_string();
            privileged = true;
            env_vars = CONTAINER_ARISTA_CEOS_ENV_VARS
                .iter()
                .map(|s| s.to_string())
                .collect();
            volumes = vec![format!("{}:{}", ztp_volume.src, ztp_volume.dst)];
            commands = CONTAINER_ARISTA_CEOS_COMMANDS
                .iter()
                .map(|s| s.to_string())
                .collect();

            // Update node fields for consistency
            node.image = Some(image.clone());
            node.privileged = Some(privileged);
            node.environment_variables = Some(env_vars.clone());
            node.volumes = Some(vec![ztp_volume]);
            node.commands = Some(commands.clone());
        }
        data::NodeModel::NokiaSrlinux => {
            let srlinux_config = match &custom_ztp {
                Some(config) => config.clone(),
                None => template::build_srlinux_config(
                    &node.name,
                    sherpa_user,
                    dns,
                    &mgmt_net.v4,
                    Some(node_ipv4_address),
                    node.ipv6_address,
                    mgmt_net.v6.as_ref(),
                )?,
            };
            let ztp_config = format!("{dir}/{}.json", node.name);
            let ztp_volume = topology::VolumeMount {
                src: ztp_config.clone(),
                dst: NOKIA_SRLINUX_ZTP_VOLUME_MOUNT.to_string(),
            };
            util::create_dir(&dir)?;
            util::create_file(&ztp_config, srlinux_config)?;

            image = CONTAINER_NOKIA_SRLINUX_REPO.to_string();
            privileged = true;
            user = Some(CONTAINER_NOKIA_SRLINUX_USER.to_string());
            env_vars = CONTAINER_NOKIA_SRLINUX_ENV_VARS
                .iter()
                .map(|s| s.to_string())
                .collect();
            volumes = vec![format!("{}:{}", ztp_volume.src, ztp_volume.dst)];
            commands = CONTAINER_NOKIA_SRLINUX_COMMANDS
                .iter()
                .map(|s| s.to_string())
                .collect();

            // Update node fields
            node.image = Some(image.clone());
            node.privileged = Some(privileged);
            node.user = user.clone();
            node.environment_variables = Some(env_vars.clone());
            node.volumes = Some(vec![ztp_volume]);
            node.commands = Some(commands.clone());
        }
        data::NodeModel::FrrLinux => {
            // Render daemons file
            let daemons_template = template::FrrDaemonsTemplate {};
            let rendered_daemons = daemons_template.render()?;
            let daemons_file = format!("{dir}/daemons");

            // Render FRR config
            let rendered_config = match &custom_ztp {
                Some(config) => config.clone(),
                None => {
                    let frr_template = template::FrrZtpTemplate {
                        hostname: node.name.clone(),
                        mgmt_ipv4: mgmt_net.v4.clone(),
                        mgmt_ipv4_address: Some(node_ipv4_address),
                        mgmt_ipv6_address: node.ipv6_address,
                        mgmt_ipv6: mgmt_net.v6.clone(),
                    };
                    frr_template.render()?
                }
            };
            let config_file = format!("{dir}/frr.conf");

            // Render startup script
            let startup_template = template::FrrStartupTemplate {
                hostname: node.name.clone(),
                user: sherpa_user.clone(),
            };
            let rendered_startup = startup_template.render()?;
            let startup_file = format!("{dir}/sherpa-init.sh");

            util::create_dir(&dir)?;
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

            image = CONTAINER_FRR_REPO.to_string();
            privileged = true;
            env_vars = CONTAINER_FRR_ENV_VARS
                .iter()
                .map(|s| s.to_string())
                .collect();
            volumes = vec![
                format!("{}:{}", daemons_volume.src, daemons_volume.dst),
                format!("{}:{}", config_volume.src, config_volume.dst),
                format!("{}:{}", startup_volume.src, startup_volume.dst),
            ];
            commands = vec!["/bin/sh".to_string(), "/tmp/sherpa-init.sh".to_string()];

            node.image = Some(image.clone());
            node.privileged = Some(privileged);
            node.environment_variables = Some(env_vars.clone());
            node.volumes = Some(vec![daemons_volume, config_volume, startup_volume]);
            node.commands = Some(commands.clone());
        }
        data::NodeModel::GitlabCe => {
            image = CONTAINER_GITLAB_CE_REPO.to_string();
            commands = CONTAINER_GITLAB_CE_COMMANDS
                .iter()
                .map(|s| s.to_string())
                .collect();
            // GitLab CE requires a larger /dev/shm for bundled PostgreSQL
            if shm_size.is_none() {
                shm_size = Some(CONTAINER_GITLAB_CE_SHM_SIZE * 1024 * 1024);
            }

            node.image = Some(image.clone());
            node.shm_size = shm_size;
            node.commands = Some(commands.clone());
        }
        data::NodeModel::SurrealDb => {
            image = CONTAINER_SURREAL_DB_REPO.to_string();
            commands = CONTAINER_SURREAL_DB_COMMANDS
                .iter()
                .map(|s| s.to_string())
                .collect();

            node.image = Some(image.clone());
            node.commands = Some(commands.clone());
        }
        data::NodeModel::MongoDb => {
            image = CONTAINER_MONGO_DB_REPO.to_string();
            commands = CONTAINER_MONGO_DB_COMMANDS
                .iter()
                .map(|s| s.to_string())
                .collect();

            node.image = Some(image.clone());
            node.commands = Some(commands.clone());
        }
        data::NodeModel::NatsBus => {
            image = CONTAINER_NATS_REPO.to_string();
            commands = CONTAINER_NATS_COMMANDS
                .iter()
                .map(|s| s.to_string())
                .collect();

            node.image = Some(image.clone());
            node.commands = Some(commands.clone());
        }
        data::NodeModel::ForgejoForge => {
            image = CONTAINER_FORGEJO_REPO.to_string();
            commands = CONTAINER_FORGEJO_COMMANDS
                .iter()
                .map(|s| s.to_string())
                .collect();

            node.image = Some(image.clone());
            node.commands = Some(commands.clone());
        }
        data::NodeModel::HashicorpVault => {
            let rendered_config = match &custom_ztp {
                Some(config) => config.clone(),
                None => {
                    let vault_template = template::VaultConfigTemplate {
                        node_name: node.name.clone(),
                    };
                    vault_template.render()?
                }
            };
            let config_file = format!("{dir}/config.hcl");
            let config_volume = topology::VolumeMount {
                src: config_file.clone(),
                dst: VAULT_ZTP_CONFIG_MOUNT.to_string(),
            };
            util::create_dir(&dir)?;
            util::create_file(&config_file, rendered_config)?;

            image = CONTAINER_HASHICORP_VAULT_REPO.to_string();
            env_vars = CONTAINER_HASHICORP_VAULT_ENV_VARS
                .iter()
                .map(|s| s.to_string())
                .collect();
            volumes = vec![format!("{}:{}", config_volume.src, config_volume.dst)];
            commands = CONTAINER_HASHICORP_VAULT_COMMANDS
                .iter()
                .map(|s| s.to_string())
                .collect();
            capabilities = model_capabilities(&node.model);

            node.image = Some(image.clone());
            node.environment_variables = Some(env_vars.clone());
            node.volumes = Some(vec![config_volume]);
            node.commands = Some(commands.clone());
        }
        _ => {}
    }

    // Volumes from node manifest (non-model-specific ones)
    if let Some(ref node_volumes) = node.volumes {
        let manifest_volumes: Vec<String> = node_volumes
            .iter()
            .map(|v| format!("{}:{}", v.src, v.dst))
            .collect();
        // For non-model-specific nodes, use the manifest volumes
        if !matches!(
            node.model,
            data::NodeModel::AristaCeos
                | data::NodeModel::NokiaSrlinux
                | data::NodeModel::FrrLinux
                | data::NodeModel::GitlabCe
                | data::NodeModel::SurrealDb
                | data::NodeModel::HashicorpVault
        ) {
            volumes = manifest_volumes;
        }
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

    let ztp_record = data::ZtpRecord {
        node_name: node.name.clone(),
        config_file: format!("{}.conf", node.name),
        ipv4_address: node_ipv4_address,
        ipv6_address: None,
        mac_address: String::new(),
        ztp_method: node_image.ztp_method.clone(),
        ssh_port: Some(SSH_PORT),
    };

    Ok(ContainerZtpResult {
        image,
        env_vars,
        volumes,
        commands,
        capabilities,
        privileged,
        shm_size,
        user,
        ztp_record,
    })
}

// ============================================================================
// VM ZTP Generation
// ============================================================================

/// Generate ZTP configuration for a VM node.
/// Returns ZTP record, clone disks, disk definitions, MAC address, and QEMU commands.
#[allow(clippy::too_many_arguments)]
#[instrument(
    level = "debug",
    skip(
        node,
        node_image,
        sherpa_user,
        mgmt_net,
        dns,
        progress,
        cert_paths,
        guest_account
    )
)]
pub(crate) fn generate_vm_ztp(
    node: &mut topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_id: &str,
    lab_dir: &str,
    tftp_dir: &str,
    images_dir: &str,
    mgmt_net: &data::SherpaNetwork,
    node_ipv4_address: Ipv4Addr,
    sherpa_user: &data::User,
    dns: &data::Dns,
    progress: &ProgressSender,
    mgmt_mac: Option<&str>,
    cert_paths: Option<&NodeCertPaths>,
    guest_account: Option<&GuestAccount>,
) -> Result<VmZtpResult> {
    let guest_account = guest_account.filter(|_| guest_account::supports_guest_account(node_image));
    if guest_account.is_some() {
        guest_account::validate_node(node, node_image)?;
    }
    let node_name_with_lab = format!("{}-{}", node.name, lab_id);
    let hdd_bus = node_image.hdd_bus.clone();
    let cdrom_bus = node_image.cdrom_bus.clone();

    // Reuse existing MAC address if provided, otherwise generate a new one
    let mac_address = mgmt_mac
        .map(|m| m.to_string())
        .unwrap_or_else(|| util::random_mac(KVM_OUI));

    let mut clone_disks: Vec<data::CloneDisk> = vec![];
    let mut disks: Vec<data::NodeDisk> = vec![];

    // Build VM boot disk clone info
    let filename = util::image_filename(&node_image.kind, node_image.boot_mode.as_ref());
    let src_boot_disk = format!(
        "{}/{}/{}/{}",
        images_dir, node_image.model, node_image.version, filename
    );
    let dst_boot_disk = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-hdd.qcow2");

    clone_disks.push(data::CloneDisk {
        src: src_boot_disk,
        dst: dst_boot_disk.clone(),
        disk_size: node.boot_disk_size,
        method: node.boot_disk_clone_method.unwrap_or_default(),
    });

    // Handle CDROM ISO
    let (mut src_cdrom_iso, mut dst_cdrom_iso) = match &node_image.cdrom {
        Some(src_iso) => {
            let src = format!(
                "{}/{}/{}/{}",
                images_dir, node_image.model, ARISTA_ABOOT_DIR, src_iso
            );
            let dst = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}.iso");
            (Some(src), Some(dst))
        }
        None => (None, None),
    };

    let (mut src_config_disk, mut dst_config_disk): (Option<String>, Option<String>) = (None, None);
    let (mut src_usb_disk, mut dst_usb_disk): (Option<String>, Option<String>) = (None, None);
    let (mut src_ignition_disk, mut dst_ignition_disk): (Option<String>, Option<String>) =
        (None, None);

    if node_image.ztp_enable {
        // Validate custom ZTP config support
        if node.ztp_config.is_some() {
            match node_image.ztp_method {
                data::ZtpMethod::CloudInit
                | data::ZtpMethod::Ignition
                | data::ZtpMethod::Http
                | data::ZtpMethod::None => {
                    bail!(
                        "Custom ZTP config is not supported for node '{}' with ZTP method '{:?}'. \
                         Only Tftp, Cdrom, Disk, Usb, and Volume methods support custom ZTP config files.",
                        node.name,
                        node_image.ztp_method
                    );
                }
                _ => {}
            }
        }

        let custom_ztp = take_custom_ztp_config(node, progress)?;

        match node_image.ztp_method {
            data::ZtpMethod::CloudInit => {
                generate_cloud_init_ztp(
                    node,
                    node_image,
                    lab_dir,
                    mgmt_net,
                    node_ipv4_address,
                    &mac_address,
                    progress,
                    cert_paths,
                    guest_account,
                )?;
                src_cdrom_iso = Some(format!("{lab_dir}/{}/{ZTP_ISO}", node.name));
                dst_cdrom_iso = Some(format!(
                    "{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}.iso"
                ));
            }
            data::ZtpMethod::Tftp => {
                generate_tftp_ztp(
                    node,
                    node_image,
                    tftp_dir,
                    sherpa_user,
                    dns,
                    mgmt_net,
                    node_ipv4_address,
                    &custom_ztp,
                    progress,
                )?;
            }
            data::ZtpMethod::NoCloud => {
                generate_nocloud_ztp(
                    node,
                    lab_dir,
                    mgmt_net,
                    node_ipv4_address,
                    &mac_address,
                    custom_ztp.as_deref(),
                    progress,
                )?;
                src_cdrom_iso = Some(format!("{lab_dir}/{}/{ZTP_ISO}", node.name));
                dst_cdrom_iso = Some(format!(
                    "{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}.iso"
                ));
            }
            data::ZtpMethod::Cdrom => {
                generate_cdrom_ztp(
                    node,
                    node_image,
                    lab_dir,
                    sherpa_user,
                    dns,
                    mgmt_net,
                    node_ipv4_address,
                    &custom_ztp,
                    progress,
                )?;
                src_cdrom_iso = Some(format!("{lab_dir}/{}/{ZTP_ISO}", node.name));
                dst_cdrom_iso = Some(format!(
                    "{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}.iso"
                ));
            }
            data::ZtpMethod::Disk => {
                let (src_cfg, dst_cfg) = generate_disk_ztp(
                    node,
                    node_image,
                    lab_dir,
                    images_dir,
                    sherpa_user,
                    dns,
                    mgmt_net,
                    node_ipv4_address,
                    &node_name_with_lab,
                    &custom_ztp,
                    progress,
                )?;
                src_config_disk = Some(src_cfg);
                dst_config_disk = Some(dst_cfg);
            }
            data::ZtpMethod::Usb => {
                let (src_usb, dst_usb) = generate_usb_ztp(
                    node,
                    node_image,
                    lab_dir,
                    images_dir,
                    sherpa_user,
                    dns,
                    mgmt_net,
                    node_ipv4_address,
                    &node_name_with_lab,
                    &custom_ztp,
                    progress,
                )?;
                src_usb_disk = Some(src_usb);
                dst_usb_disk = Some(dst_usb);
            }
            data::ZtpMethod::Http => {
                generate_http_ztp(
                    node,
                    node_image,
                    lab_dir,
                    sherpa_user,
                    mgmt_net,
                    node_ipv4_address,
                    progress,
                )?;
            }
            data::ZtpMethod::Ignition => {
                let (src_ign, dst_ign, src_cfg, dst_cfg) = generate_ignition_ztp(
                    node,
                    node_image,
                    lab_dir,
                    images_dir,
                    sherpa_user,
                    mgmt_net,
                    node_ipv4_address,
                    &node_name_with_lab,
                    progress,
                    cert_paths,
                    guest_account,
                )?;
                src_ignition_disk = Some(src_ign);
                dst_ignition_disk = Some(dst_ign);
                src_config_disk = Some(src_cfg);
                dst_config_disk = Some(dst_cfg);
            }
            data::ZtpMethod::Volume => {
                let _ = progress.send_status(
                    format!(
                        "ZTP method Volume is not applicable to VMs, skipping: {}",
                        node.name
                    ),
                    StatusKind::Info,
                );
            }
            data::ZtpMethod::Ipxe => {
                let _ = progress.send_status(
                    format!(
                        "ZTP method iPXE is not yet implemented for VM: {}",
                        node.name
                    ),
                    StatusKind::Info,
                );
            }
            data::ZtpMethod::None => {
                let _ = progress.send_status(
                    format!("ZTP disabled for VM: {}", node.name),
                    StatusKind::Info,
                );
            }
        }
    }

    // Build disk list: CDROM ISO
    if let (Some(src_iso), Some(dst_iso)) = (src_cdrom_iso, dst_cdrom_iso) {
        clone_disks.push(data::CloneDisk {
            src: src_iso,
            dst: dst_iso.clone(),
            disk_size: None,
            method: data::DiskImageCloneMethod::Copy,
        });
        disks.push(data::NodeDisk {
            disk_device: data::DiskDevices::Cdrom,
            driver_name: data::DiskDrivers::Qemu,
            driver_format: data::DiskFormats::Raw,
            src_file: dst_iso,
            target_dev: data::DiskTargets::target(&cdrom_bus, disks.len() as u8)?,
            target_bus: cdrom_bus.clone(),
        });
    }

    // Boot disk
    disks.push(data::NodeDisk {
        disk_device: data::DiskDevices::File,
        driver_name: data::DiskDrivers::Qemu,
        driver_format: data::DiskFormats::Qcow2,
        src_file: dst_boot_disk,
        target_dev: data::DiskTargets::target(&hdd_bus, disks.len() as u8)?,
        target_bus: hdd_bus.clone(),
    });

    // Config disk
    if let (Some(src_disk), Some(dst_disk)) = (src_config_disk, dst_config_disk) {
        clone_disks.push(data::CloneDisk {
            src: src_disk,
            dst: dst_disk.clone(),
            disk_size: None,
            method: data::DiskImageCloneMethod::Copy,
        });
        disks.push(data::NodeDisk {
            disk_device: data::DiskDevices::File,
            driver_name: data::DiskDrivers::Qemu,
            driver_format: data::DiskFormats::Raw,
            src_file: dst_disk,
            target_dev: data::DiskTargets::target(&hdd_bus, disks.len() as u8)?,
            target_bus: hdd_bus.clone(),
        });
    }

    // USB disk
    if let (Some(src_disk), Some(dst_disk)) = (src_usb_disk, dst_usb_disk) {
        clone_disks.push(data::CloneDisk {
            src: src_disk,
            dst: dst_disk.clone(),
            disk_size: None,
            method: data::DiskImageCloneMethod::Copy,
        });
        // USB and SATA share the sd* target namespace, so count existing sd* targets
        // to avoid duplicate target device names.
        let usb_target_index = disks
            .iter()
            .filter(|d| matches!(d.target_bus, data::DiskBuses::Sata | data::DiskBuses::Usb))
            .count() as u8;
        let usb_target_dev = data::DiskTargets::target(&data::DiskBuses::Usb, usb_target_index)?;

        disks.push(data::NodeDisk {
            disk_device: data::DiskDevices::File,
            driver_name: data::DiskDrivers::Qemu,
            driver_format: data::DiskFormats::Raw,
            src_file: dst_disk,
            target_dev: usb_target_dev,
            target_bus: data::DiskBuses::Usb,
        });
    }

    // Ignition config
    if let (Some(src_ignition), Some(dst_ignition)) =
        (src_ignition_disk.clone(), dst_ignition_disk.clone())
    {
        clone_disks.push(data::CloneDisk {
            src: src_ignition,
            dst: dst_ignition,
            disk_size: None,
            method: data::DiskImageCloneMethod::Copy,
        });
    }

    // Build QEMU commands
    let qemu_commands = match node_image.model {
        data::NodeModel::JuniperVrouter => data::QemuCommand::juniper_vrouter(),
        data::NodeModel::JuniperVswitch => data::QemuCommand::juniper_vswitch(),
        data::NodeModel::JuniperVsrxv3 => data::QemuCommand::juniper_vsrxv3(),
        data::NodeModel::JuniperVevolved => data::QemuCommand::juniper_vevolved(),
        data::NodeModel::FlatcarLinux => {
            if let Some(ref dst_ignition) = dst_ignition_disk {
                data::QemuCommand::ignition_config(dst_ignition)
            } else {
                vec![]
            }
        }
        _ => vec![],
    };

    let ztp_record = data::ZtpRecord {
        node_name: node.name.clone(),
        config_file: format!("{}.conf", node.name),
        ipv4_address: node_ipv4_address,
        ipv6_address: None,
        mac_address: mac_address.to_string(),
        ztp_method: node_image.ztp_method.clone(),
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

// ============================================================================
// TLS Certificate Helpers
// ============================================================================

/// Build cloud-init write_files entries for TLS certificates
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

/// Build cloudbase-init write_files entries for TLS certificates (Windows)
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
            path: format!(r"{}\ca.crt", target_dir),
            content: ca_cert_content,
            permissions: "0644".to_string(),
        },
        template::CloudbaseWriteFile {
            path: format!(r"{}\node.crt", target_dir),
            content: node_cert_content,
            permissions: "0644".to_string(),
        },
        template::CloudbaseWriteFile {
            path: format!(r"{}\node.key", target_dir),
            content: node_key_content,
            permissions: "0600".to_string(),
        },
    ])
}

/// Build staged files from pre-encoded text_files
fn build_cloud_init_text_files(
    text_files: &Option<Vec<topology::TextFileData>>,
) -> Vec<StagedFile> {
    text_files
        .iter()
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

/// Build cloudbase-init write_files entries from pre-encoded text_files
fn build_cloudbase_text_files(
    text_files: &Option<Vec<topology::TextFileData>>,
) -> Vec<template::CloudbaseWriteFile> {
    text_files
        .iter()
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
            mode: 644,
            contents: Some(template::IgnitionFileContents::new(&format!(
                "data:;base64,{ca_b64}"
            ))),
            ..Default::default()
        },
        template::IgnitionFile {
            path: format!("{target_dir}/node.crt"),
            mode: 644,
            contents: Some(template::IgnitionFileContents::new(&format!(
                "data:;base64,{cert_b64}"
            ))),
            ..Default::default()
        },
        template::IgnitionFile {
            path: format!("{target_dir}/node.key"),
            mode: 600,
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
            script.push_str(&format!("export {}='{}'\n", key, escaped_value));
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
            "chown -R {quoted_user}:\"$(id -gn {quoted_user})\" {}",
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
    text_files: &Option<Vec<topology::TextFileData>>,
) -> Vec<template::IgnitionDirectory> {
    let mut directories: Vec<template::IgnitionDirectory> = Vec::new();
    for file in text_files.iter().flatten() {
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
                mode: 755,
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
            mode: 600,
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
            mode: 644,
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
                "powershell -Command \"[System.Environment]::SetEnvironmentVariable('{}', '{}', 'Machine')\"",
                key, escaped_value
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
                "Windows file path '{}' contains '\"' or '%', which cannot be used in a path on a Windows node",
                file.path
            );
        }
        commands.push(format!(
            "icacls \"{}\" /inheritance:r /grant {WINDOWS_ADMINISTRATORS_SID}:F /grant {WINDOWS_SYSTEM_SID}:F",
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
fn cloud_init_ipv6_address(model: &data::NodeModel, address: Option<Ipv6Addr>) -> Option<Ipv6Addr> {
    match model {
        data::NodeModel::OpenBsd => None,
        _ => address,
    }
}

/// Whether cloud-init should manage a node's `/etc/hosts`.
///
/// OpenBSD does not: cloud-init ships no hosts template for it, so the module fails and
/// cloud-init reports an error. The node's name resolves through the lab DNS instead.
fn cloud_init_manage_etc_hosts(model: &data::NodeModel) -> bool {
    !matches!(model, data::NodeModel::OpenBsd)
}

/// The doas rule giving `wheel` admin rights on OpenBSD, where doas is the native tool: the
/// stock `/etc/examples/doas.conf` rule, without a password, since the logins have none. The
/// image has no `/etc/doas.conf`.
fn cloud_init_doas_file(model: &data::NodeModel) -> Option<template::CloudInitWriteFile> {
    if *model != data::NodeModel::OpenBsd {
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

/// The cloud-init `sudo` rule for admin logins, except on OpenBSD, which uses doas.
fn cloud_init_sudo_rule(model: &data::NodeModel) -> Option<String> {
    match model {
        data::NodeModel::OpenBsd => None,
        _ => Some(NODE_CLOUD_INIT_SUDO_RULE.to_owned()),
    }
}

#[instrument(level = "debug")]
fn cloud_init_user_settings(
    model: &data::NodeModel,
    os_variant: &data::OsVariant,
) -> (&'static str, &'static str) {
    match (model, os_variant) {
        (data::NodeModel::OmarchyLinux, _) => ("wheel", "/bin/bash"),
        (_, data::OsVariant::Bsd) => ("wheel", "/bin/sh"),
        _ => ("sudo", "/bin/bash"),
    }
}

#[allow(clippy::too_many_arguments)]
fn generate_cloud_init_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_dir: &str,
    mgmt_net: &data::SherpaNetwork,
    node_ipv4_address: Ipv4Addr,
    mac_address: &str,
    progress: &ProgressSender,
    cert_paths: Option<&NodeCertPaths>,
    guest_account: Option<&GuestAccount>,
) -> Result<()> {
    let _ = progress.send_status(
        format!("Creating Cloud-Init config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let dir = format!("{lab_dir}/{}", node.name);
    let mut cloud_init_user = template::CloudInitUser::sherpa()?;

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
        | data::NodeModel::KaliLinux
        | data::NodeModel::InfrahubServer
        | data::NodeModel::JenkinsServer
        | data::NodeModel::NautobotServer
        | data::NodeModel::NetboxServer
        | data::NodeModel::SignozServer
        | data::NodeModel::VirtServer
        | data::NodeModel::FreeBsd
        | data::NodeModel::OpenBsd => {
            let (admin_group, shell) =
                cloud_init_user_settings(&node.model, &node_image.os_variant);
            cloud_init_user.groups = vec![admin_group.to_owned()];
            cloud_init_user.shell = shell.to_owned();
            cloud_init_user.sudo = cloud_init_sudo_rule(&node.model);

            let mut write_files = match cert_paths {
                Some(certs) => build_cloud_init_cert_files(certs, NODE_CERTS_DIR)?,
                None => vec![],
            };

            // Stage text_files and the guest login's environment file, and move them into
            // place first in runcmd, once their owners exist.
            let guest_username = guest_account::guest_username(guest_account);
            let mut staged_files = build_cloud_init_text_files(&node.text_files);
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
            if let Some(doas_file) = cloud_init_doas_file(&node.model) {
                write_files.push(doas_file);
            }

            let cloud_init_write_files = if write_files.is_empty() {
                None
            } else {
                Some(write_files)
            };

            let cloud_init_runcmd = if runcmd.is_empty() {
                None
            } else {
                Some(runcmd)
            };

            let cloud_init_config = template::CloudInitConfig {
                hostname: node.name.clone(),
                fqdn: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
                manage_etc_hosts: cloud_init_manage_etc_hosts(&node.model),
                ssh_pwauth: true,
                users: {
                    let mut users = vec![cloud_init_user];
                    if let Some(account) = guest_account {
                        let mut user = match node_image.os_variant {
                            data::OsVariant::Bsd => guest_account::bsd_cloud_init_user(account),
                            _ => guest_account::cloud_init_user(account),
                        };
                        user.sudo = cloud_init_sudo_rule(&node.model);
                        users.push(user);
                    }
                    users
                },
                write_files: cloud_init_write_files,
                runcmd: cloud_init_runcmd,
                ..Default::default()
            };
            let user_data_config = cloud_init_config.to_string()?;

            let meta_data_obj = template::MetaDataConfig {
                instance_id: format!("iid-{}", node.name),
                local_hostname: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
                ..Default::default()
            };
            let meta_data_config = meta_data_obj.to_string()?;

            let user_data = format!("{dir}/{CLOUD_INIT_USER_DATA}");
            let meta_data = format!("{dir}/{CLOUD_INIT_META_DATA}");
            let network_config = format!("{dir}/{CLOUD_INIT_NETWORK_CONFIG}");

            util::create_dir(&dir)?;
            util::create_file(&user_data, user_data_config)?;
            util::create_file(&meta_data, meta_data_config)?;

            let ztp_interface = template::CloudInitNetwork::ztp_interface(
                node_ipv4_address,
                mac_address.to_string(),
                mgmt_net.v4.clone(),
                cloud_init_ipv6_address(&node.model, node.ipv6_address),
                mgmt_net.v6.as_ref(),
            );
            let cloud_network_config = ztp_interface.to_string()?;
            util::create_file(&network_config, cloud_network_config)?;

            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
        }
        data::NodeModel::DevboxWindows | data::NodeModel::WindowsServer => {
            let cloudbase_user = template::CloudbaseInitUser::sherpa()?;

            let ssh_key = util::get_ssh_public_key(SHERPA_SSH_PUBLIC_KEY_PATH)?;
            let ssh_key_str = format!("{} {}", ssh_key.algorithm, ssh_key.key);

            let admin_keys_path = r"C:\ProgramData\ssh\administrators_authorized_keys";

            let mut write_files = vec![template::CloudbaseWriteFile {
                path: admin_keys_path.to_string(),
                content: ssh_key_str.clone(),
                permissions: "0644".to_string(),
            }];
            if let Some(certs) = cert_paths {
                write_files.extend(build_cloudbase_cert_files(certs, NODE_CERTS_DIR_WINDOWS)?);
            }

            // Inject text_files into write_files
            write_files.extend(build_cloudbase_text_files(&node.text_files));

            // Inject startup_scripts for Windows
            let mut runcmd = vec![format!(
                "icacls \"{}\" /inheritance:r /grant \"Administrators:F\" /grant \"SYSTEM:F\"",
                admin_keys_path
            )];
            runcmd.extend(
                build_windows_private_file_acls(&write_files)
                    .with_context(|| format!("Failed to write files for node '{}'", node.name))?,
            );
            runcmd.push("powershell -Command \"Restart-Service sshd\"".to_string());

            if let Some(ref scripts) = node.startup_scripts {
                for script in scripts {
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
                        "powershell -ExecutionPolicy Bypass -File \"{}\"",
                        target_path
                    ));
                }
            }

            // Inject environment_variables as machine-level env vars
            if let Some(ref env_vars) = node.environment_variables
                && !env_vars.is_empty()
            {
                runcmd.extend(build_windows_env_commands(env_vars));
            }

            let cloudbase_config = template::CloudbaseInitConfig {
                set_hostname: node.name.clone(),
                users: vec![cloudbase_user],
                write_files,
                runcmd,
            };
            let user_data_config = cloudbase_config.to_string()?;

            let meta_data_obj = template::MetaDataConfig {
                instance_id: format!("iid-{}", node.name),
                local_hostname: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
                public_keys: vec![ssh_key_str],
            };
            let meta_data_config = meta_data_obj.to_string()?;

            let user_data = format!("{dir}/{CLOUD_INIT_USER_DATA}");
            let meta_data = format!("{dir}/{CLOUD_INIT_META_DATA}");
            let network_config = format!("{dir}/{CLOUD_INIT_NETWORK_CONFIG}");

            util::create_dir(&dir)?;
            util::create_file(&user_data, user_data_config)?;
            util::create_file(&meta_data, meta_data_config)?;

            let ztp_interface = template::CloudbaseInitNetwork::ztp_interface(
                node_ipv4_address,
                mac_address.to_string(),
                mgmt_net.v4.clone(),
                node.ipv6_address,
                mgmt_net.v6.as_ref(),
            );
            let cloud_network_config = ztp_interface.to_string()?;
            util::create_file(&network_config, cloud_network_config)?;

            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
        }
        data::NodeModel::AlpineLinux => {
            let meta_data = template::MetaDataConfig {
                instance_id: format!("iid-{}", node.name),
                local_hostname: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
                ..Default::default()
            };
            cloud_init_user.shell = "/bin/sh".to_string();
            cloud_init_user.groups = vec!["wheel".to_string()];

            let mut write_files = match cert_paths {
                Some(certs) => build_cloud_init_cert_files(certs, NODE_CERTS_DIR)?,
                None => vec![],
            };

            // Stage text_files and the guest login's environment file, and move them into
            // place first in runcmd, once their owners exist.
            let guest_username = guest_account::guest_username(guest_account);
            let mut staged_files = build_cloud_init_text_files(&node.text_files);
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
            if let Some(account) = guest_account {
                write_files.push(guest_account::doas_file(account));
            }

            let cloud_init_write_files = if write_files.is_empty() {
                None
            } else {
                Some(write_files)
            };

            let cloud_init_runcmd = if runcmd.is_empty() {
                None
            } else {
                Some(runcmd)
            };

            let cloud_init_config = template::CloudInitConfig {
                hostname: node.name.clone(),
                fqdn: format!("{}.{}", node.name, SHERPA_DOMAIN_NAME),
                manage_etc_hosts: cloud_init_manage_etc_hosts(&node.model),
                ssh_pwauth: true,
                // Alpine's network renderer leaves DNS to a resolvconf tool the image lacks.
                manage_resolv_conf: Some(true),
                resolv_conf: Some(template::CloudInitResolvConf::lab_dns(
                    mgmt_net.v4.boot_server,
                )),
                packages: None,
                users: {
                    let mut users = vec![cloud_init_user];
                    if let Some(account) = guest_account {
                        users.push(guest_account::alpine_cloud_init_user(account));
                    }
                    users
                },
                write_files: cloud_init_write_files,
                runcmd: cloud_init_runcmd,
            };
            let meta_data_config = meta_data.to_string()?;
            let user_data_config = cloud_init_config.to_string()?;

            let user_data = format!("{dir}/{CLOUD_INIT_USER_DATA}");
            let meta_data = format!("{dir}/{CLOUD_INIT_META_DATA}");
            let network_config = format!("{dir}/{CLOUD_INIT_NETWORK_CONFIG}");

            util::create_dir(&dir)?;
            util::create_file(&user_data, user_data_config)?;
            util::create_file(&meta_data, meta_data_config)?;

            let ztp_interface = template::CloudInitNetwork::ztp_interface(
                node_ipv4_address,
                mac_address.to_string(),
                mgmt_net.v4.clone(),
                node.ipv6_address,
                mgmt_net.v6.as_ref(),
            );
            let cloud_network_config = ztp_interface.to_string()?;
            util::create_file(&network_config, cloud_network_config)?;

            util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
        }
        _ => {
            bail!(
                "Cloud-Init ZTP method not supported for {}",
                node_image.model
            );
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn generate_tftp_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    tftp_dir: &str,
    sherpa_user: &data::User,
    dns: &data::Dns,
    mgmt_net: &data::SherpaNetwork,
    node_ipv4_address: Ipv4Addr,
    custom_ztp: &Option<String>,
    progress: &ProgressSender,
) -> Result<()> {
    let _ = progress.send_status(
        format!("Creating TFTP ZTP config for VM: {}", node.name),
        StatusKind::Progress,
    );

    if let Some(custom_config) = custom_ztp {
        let ztp_config = format!("{tftp_dir}/{}.conf", node.name);
        util::create_file(&ztp_config, custom_config.clone())?;
    } else {
        match node.model {
            data::NodeModel::AristaVeos => {
                let arista_template = template::AristaVeosZtpTemplate {
                    hostname: node.name.clone(),
                    user: sherpa_user.clone(),
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
                    mgmt_interface: node_image.management_interface.to_string(),
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

#[allow(clippy::too_many_arguments)]
fn generate_cdrom_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_dir: &str,
    sherpa_user: &data::User,
    dns: &data::Dns,
    mgmt_net: &data::SherpaNetwork,
    node_ipv4_address: Ipv4Addr,
    custom_ztp: &Option<String>,
    progress: &ProgressSender,
) -> Result<()> {
    let _ = progress.send_status(
        format!("Creating CDROM ZTP config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let dir = format!("{lab_dir}/{}", node.name);

    if let Some(custom_config) = custom_ztp {
        let config_filename = ztp_config_filename(&node.model)?;
        let ztp_config = format!("{dir}/{config_filename}");
        util::create_dir(&dir)?;
        util::create_file(&ztp_config, custom_config.clone())?;
        util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
    } else {
        let mut user = sherpa_user.clone();

        match node.model {
            data::NodeModel::CiscoCsr1000v
            | data::NodeModel::CiscoCat8000v
            | data::NodeModel::CiscoCat9000v => {
                let license_boot_command = if node.model == data::NodeModel::CiscoCat8000v {
                    Some("license boot level network-premier addon dna-premier".to_string())
                } else if node.model == data::NodeModel::CiscoCat9000v {
                    Some("license boot level network-advantage addon dna-advantage".to_string())
                } else {
                    None
                };

                let key_hash = util::pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                user.ssh_public_key.key = key_hash;
                let t = template::CiscoIosXeZtpTemplate {
                    hostname: node.name.clone(),
                    user,
                    mgmt_interface: node_image.management_interface.to_string(),
                    dns: dns.clone(),
                    license_boot_command,
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                let rendered_template = t.render()?;
                let c = CISCO_IOSXE_ZTP_CONFIG.replace("-", "_");
                let ztp_config = format!("{dir}/{c}");
                util::create_dir(&dir)?;
                util::create_file(&ztp_config, rendered_template)?;
                util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
            }
            data::NodeModel::CiscoAsav => {
                let key_hash = util::pub_ssh_key_to_sha256_hash(&user.ssh_public_key.key)?;
                user.ssh_public_key.key = key_hash;
                let t = template::CiscoAsavZtpTemplate {
                    hostname: node.name.clone(),
                    user,
                    dns: dns.clone(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                let rendered_template = t.render()?;
                let ztp_config = format!("{dir}/{CISCO_ASAV_ZTP_CONFIG}");
                util::create_dir(&dir)?;
                util::create_file(&ztp_config, rendered_template)?;
                util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
            }
            data::NodeModel::CiscoNexus9300v => {
                let t = template::CiscoNxosZtpTemplate {
                    hostname: node.name.clone(),
                    user,
                    dns: dns.clone(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                let rendered_template = t.render()?;
                let ztp_config = format!("{dir}/{CISCO_NXOS_ZTP_CONFIG}");
                util::create_dir(&dir)?;
                util::create_file(&ztp_config, rendered_template)?;
                util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
            }
            data::NodeModel::CiscoIosxrv9000 => {
                let t = template::CiscoIosxrZtpTemplate {
                    hostname: node.name.clone(),
                    user,
                    dns: dns.clone(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                let rendered_template = t.render()?;
                let ztp_config = format!("{dir}/{CISCO_IOSXR_ZTP_CONFIG}");
                util::create_dir(&dir)?;
                util::create_file(&ztp_config, rendered_template)?;
                util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
            }
            data::NodeModel::CiscoFtdv => {
                let t = template::CiscoFtdvZtpTemplate {
                    eula: "accept".to_string(),
                    hostname: node.name.clone(),
                    admin_password: SHERPA_PASSWORD.to_string(),
                    dns1: Some(mgmt_net.v4.boot_server),
                    ipv4_mode: Some(template::CiscoFxosIpMode::Manual),
                    ipv4_addr: Some(node_ipv4_address),
                    ipv4_gw: Some(mgmt_net.v4.first),
                    ipv4_mask: Some(mgmt_net.v4.subnet_mask),
                    manage_locally: true,
                    ..Default::default()
                };
                let rendered_template = serde_json::to_string(&t)?;
                let ztp_config = format!("{dir}/{CISCO_FTDV_ZTP_CONFIG}");
                util::create_dir(&dir)?;
                util::create_file(&ztp_config, rendered_template)?;
                util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
            }
            data::NodeModel::JuniperVsrxv3
            | data::NodeModel::JuniperVrouter
            | data::NodeModel::JuniperVswitch => {
                let t = template::JunipervJunosZtpTemplate {
                    hostname: node.name.clone(),
                    user,
                    mgmt_interface: node_image.management_interface.to_string(),
                    mgmt_ipv4_address: Some(node_ipv4_address),
                    mgmt_ipv4: mgmt_net.v4.clone(),
                    mgmt_ipv6_address: node.ipv6_address,
                    mgmt_ipv6: mgmt_net.v6.clone(),
                };
                let rendered_template = t.render()?;
                let ztp_config = format!("{dir}/{JUNIPER_ZTP_CONFIG}");
                util::create_dir(&dir)?;
                util::create_file(&ztp_config, rendered_template)?;
                util::create_ztp_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
            }
            data::NodeModel::PaloaltoPanos => {
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
                let bootstrap = template::PaloAltoPanosBootstrapTemplate {
                    hostname: node.name.clone(),
                    user,
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

                util::create_panos_bootstrap_iso(&format!("{dir}/{ZTP_ISO}"), dir)?;
            }
            _ => {
                bail!("CDROM ZTP method not supported for {}", node_image.model);
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn generate_disk_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_dir: &str,
    images_dir: &str,
    sherpa_user: &data::User,
    dns: &data::Dns,
    mgmt_net: &data::SherpaNetwork,
    node_ipv4_address: Ipv4Addr,
    node_name_with_lab: &str,
    custom_ztp: &Option<String>,
    progress: &ProgressSender,
) -> Result<(String, String)> {
    let _ = progress.send_status(
        format!("Creating Disk ZTP config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let dir = format!("{lab_dir}/{}", node.name);
    let mut user = sherpa_user.clone();

    match node.model {
        data::NodeModel::CiscoIosv => {
            let rendered_template = match custom_ztp {
                Some(config) => config.clone(),
                None => {
                    let key_hash = util::pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                    user.ssh_public_key.key = key_hash;
                    let t = template::CiscoIosvZtpTemplate {
                        hostname: node.name.clone(),
                        user,
                        mgmt_interface: node_image.management_interface.to_string(),
                        dns: dns.clone(),
                        mgmt_ipv4_address: Some(node_ipv4_address),
                        mgmt_ipv4: mgmt_net.v4.clone(),
                        mgmt_ipv6_address: node.ipv6_address,
                        mgmt_ipv6: mgmt_net.v6.clone(),
                    };
                    t.render()?
                }
            };
            let c = CISCO_IOSV_ZTP_CONFIG;
            let ztp_config = format!("{dir}/{c}");
            util::create_dir(&dir)?;
            util::create_file(&ztp_config, rendered_template)?;

            let src_disk = format!(
                "{}/{}/{}",
                images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_IOSV
            );
            let dst_disk = format!("{dir}/{}-cfg.img", node.name);

            util::copy_file(&src_disk, &dst_disk)?;
            util::copy_to_dos_image(&ztp_config, &dst_disk, "/")?;

            let dst_final = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img");
            Ok((dst_disk, dst_final))
        }
        data::NodeModel::CiscoIosvl2 => {
            let rendered_template = match custom_ztp {
                Some(config) => config.clone(),
                None => {
                    let key_hash = util::pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                    user.ssh_public_key.key = key_hash;
                    let t = template::CiscoIosvl2ZtpTemplate {
                        hostname: node.name.clone(),
                        user,
                        mgmt_interface: node_image.management_interface.to_string(),
                        dns: dns.clone(),
                        mgmt_ipv4_address: Some(node_ipv4_address),
                        mgmt_ipv4: mgmt_net.v4.clone(),
                        mgmt_ipv6_address: node.ipv6_address,
                        mgmt_ipv6: mgmt_net.v6.clone(),
                    };
                    t.render()?
                }
            };
            let c = CISCO_IOSV_ZTP_CONFIG;
            let ztp_config = format!("{dir}/{c}");
            util::create_dir(&dir)?;
            util::create_file(&ztp_config, rendered_template)?;

            let src_disk = format!(
                "{}/{}/{}",
                images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_IOSV
            );
            let dst_disk = format!("{dir}/{}-cfg.img", node.name);

            util::copy_file(&src_disk, &dst_disk)?;
            util::copy_to_dos_image(&ztp_config, &dst_disk, "/")?;

            let dst_final = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img");
            Ok((dst_disk, dst_final))
        }
        data::NodeModel::CiscoIse => {
            let rendered_template = match custom_ztp {
                Some(config) => config.clone(),
                None => {
                    let t = template::CiscoIseZtpTemplate {
                        hostname: node.name.clone(),
                        user,
                        dns: dns.clone(),
                        mgmt_ipv4_address: node_ipv4_address,
                        mgmt_ipv4: mgmt_net.v4.clone(),
                        mgmt_ipv6_address: node.ipv6_address,
                        mgmt_ipv6: mgmt_net.v6.clone(),
                    };
                    t.render()?
                }
            };
            let ztp_config = format!("{dir}/{CISCO_ISE_ZTP_CONFIG}");
            util::create_dir(&dir)?;
            util::create_file(&ztp_config, rendered_template)?;

            let src_disk = format!(
                "{}/{}/{}",
                images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_ISE
            );
            let dst_disk = format!("{dir}/{node_name_with_lab}-cfg.img");

            util::copy_file(&src_disk, &dst_disk)?;
            util::copy_to_ext4_image(vec![&ztp_config], &dst_disk, "/")?;

            let dst_final = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img");
            Ok((dst_disk, dst_final))
        }
        data::NodeModel::MikrotikChr => {
            let rendered_template = match custom_ztp {
                Some(config) => config.clone(),
                None => {
                    let t = template::MikrotikRouterosZtpTemplate {
                        hostname: node.name.clone(),
                        user: sherpa_user.clone(),
                        mgmt_interface: node_image.management_interface.to_string(),
                        dns: dns.clone(),
                        mgmt_ipv4: mgmt_net.v4.clone(),
                        mgmt_ipv4_address: Some(node_ipv4_address),
                        mgmt_ipv6_address: node.ipv6_address,
                        mgmt_ipv6: mgmt_net.v6.clone(),
                    };
                    t.render()?
                }
            };
            let ztp_config = format!("{dir}/{MIKROTIK_CHR_ZTP_CONFIG}");
            util::create_dir(&dir)?;
            util::create_file(&ztp_config, rendered_template)?;

            let src_disk = format!(
                "{}/{}/{}",
                images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_FAT32
            );
            let dst_disk = format!("{dir}/{}-cfg.img", node.name);

            util::copy_file(&src_disk, &dst_disk)?;
            util::copy_to_dos_image(&ztp_config, &dst_disk, "/")?;
            util::copy_to_dos_image(SHERPA_SSH_PUBLIC_KEY_PATH, &dst_disk, "/")?;

            let dst_final = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img");
            Ok((dst_disk, dst_final))
        }
        _ => {
            bail!("Disk ZTP method not supported for {}", node_image.model);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn generate_usb_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_dir: &str,
    images_dir: &str,
    sherpa_user: &data::User,
    dns: &data::Dns,
    mgmt_net: &data::SherpaNetwork,
    node_ipv4_address: Ipv4Addr,
    node_name_with_lab: &str,
    custom_ztp: &Option<String>,
    progress: &ProgressSender,
) -> Result<(String, String)> {
    let _ = progress.send_status(
        format!("Creating USB ZTP config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let dir = format!("{lab_dir}/{}", node.name);
    let user = sherpa_user.clone();

    match node_image.model {
        data::NodeModel::CumulusLinux => {
            let rendered_template = match custom_ztp {
                Some(config) => config.clone(),
                None => {
                    let t = template::CumulusLinuxZtpTemplate {
                        hostname: node.name.clone(),
                        user,
                        dns: dns.clone(),
                        mgmt_ipv4_address: Some(node_ipv4_address),
                        mgmt_ipv4: mgmt_net.v4.clone(),
                        mgmt_ipv6_address: node.ipv6_address,
                        mgmt_ipv6: mgmt_net.v6.clone(),
                    };
                    t.render()?
                }
            };
            let ztp_config = format!("{dir}/{CUMULUS_ZTP}");
            util::create_dir(&dir)?;
            util::create_file(&ztp_config, rendered_template)?;

            let src_usb = format!(
                "{}/{}/{}",
                images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_FAT32
            );
            let dst_usb = format!("{dir}/cfg.img");

            util::copy_file(&src_usb, &dst_usb)?;
            util::copy_to_dos_image(&ztp_config, &dst_usb, "/")?;

            let dst_final = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.img");
            Ok((dst_usb, dst_final))
        }
        data::NodeModel::JuniperVevolved => {
            let rendered_template = match custom_ztp {
                Some(config) => config.clone(),
                None => {
                    let t = template::JunipervJunosZtpTemplate {
                        hostname: node.name.clone(),
                        user,
                        mgmt_interface: node_image.management_interface.to_string(),
                        mgmt_ipv4_address: Some(node_ipv4_address),
                        mgmt_ipv4: mgmt_net.v4.clone(),
                        mgmt_ipv6_address: node.ipv6_address,
                        mgmt_ipv6: mgmt_net.v6.clone(),
                    };
                    t.render()?
                }
            };
            let ztp_config = format!("{dir}/{JUNIPER_ZTP_CONFIG}");
            let ztp_config_tgz = format!("{dir}/{JUNIPER_ZTP_CONFIG_TGZ}");

            util::create_dir(&dir)?;
            util::create_file(&ztp_config, rendered_template)?;

            let src_usb = format!(
                "{}/{}/{}",
                images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_JUNOS
            );
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

#[allow(clippy::too_many_arguments)]
fn generate_http_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_dir: &str,
    sherpa_user: &data::User,
    mgmt_net: &data::SherpaNetwork,
    node_ipv4_address: Ipv4Addr,
    progress: &ProgressSender,
) -> Result<()> {
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
            };
            let ztp_user_script = format!("{dir}/sonic_ztp_user.sh");
            util::create_file(&ztp_user_script, sonic_user_template.render()?)?;
        }
        _ => {
            bail!("HTTP ZTP method not supported for {}", node_image.model);
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn generate_ignition_ztp(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_dir: &str,
    images_dir: &str,
    sherpa_user: &data::User,
    mgmt_net: &data::SherpaNetwork,
    node_ipv4_address: Ipv4Addr,
    node_name_with_lab: &str,
    progress: &ProgressSender,
    cert_paths: Option<&NodeCertPaths>,
    guest_account: Option<&GuestAccount>,
) -> Result<(String, String, String, String)> {
    let _ = progress.send_status(
        format!("Creating Ignition config for VM: {}", node.name),
        StatusKind::Progress,
    );

    let user = sherpa_user.clone();
    let dir = format!("{lab_dir}/{}", node.name);
    let dev_name = node.name.clone();

    // Build authorized keys list
    let mut authorized_keys = vec![format!(
        "{} {} {}",
        user.ssh_public_key.algorithm,
        user.ssh_public_key.key,
        user.ssh_public_key.comment.clone().unwrap_or_default()
    )];

    let manifest_authorized_keys: Vec<String> =
        node.ssh_authorized_keys.clone().unwrap_or_default();

    let manifest_authorized_key_files: Vec<String> = node
        .ssh_authorized_key_files
        .iter()
        .flatten()
        .map(|file| -> Result<String> {
            let ssh_key = util::get_ssh_public_key(&file.source)?;
            Ok(format!(
                "{} {} {}",
                ssh_key.algorithm,
                ssh_key.key,
                ssh_key.comment.unwrap_or_default()
            ))
        })
        .collect::<Result<Vec<String>>>()?;

    authorized_keys.extend(manifest_authorized_keys);
    authorized_keys.extend(manifest_authorized_key_files);

    let ignition_user = template::IgnitionUser {
        name: user.username.clone(),
        password_hash: Some(SHERPA_PASSWORD_HASH.to_owned()),
        home_dir: None,
        shell: None,
        ssh_authorized_keys: authorized_keys,
        groups: vec!["wheel".to_owned(), "docker".to_owned()],
    };

    let hostname_file = template::IgnitionFile {
        path: "/etc/hostname".to_owned(),
        mode: 644,
        contents: Some(template::IgnitionFileContents::new(&format!(
            "data:,{dev_name}"
        ))),
        ..Default::default()
    };

    let disable_update = template::IgnitionFile::disable_updates();
    let sudo_config_base64 =
        util::base64_encode(&format!("{SHERPA_USERNAME} ALL=(ALL) NOPASSWD: ALL"));
    let sudo_config_file = template::IgnitionFile {
        path: format!("/etc/sudoers.d/{SHERPA_USERNAME}"),
        mode: 440,
        contents: Some(template::IgnitionFileContents::new(&format!(
            "data:;base64,{sudo_config_base64}"
        ))),
        ..Default::default()
    };

    let manifest_text_files: Vec<template::IgnitionFile> = node
        .text_files
        .iter()
        .flatten()
        .map(|file| template::IgnitionFile {
            path: file.dst.clone(),
            mode: file.permissions,
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
        .collect();

    let manifest_binary_disk_files = node.binary_files.clone().unwrap_or_default();

    let manifest_systemd_units: Vec<template::IgnitionUnit> = node
        .systemd_units
        .iter()
        .flatten()
        .map(|file| {
            let file_contents = util::load_file(file.source.as_str())?;
            Ok(template::IgnitionUnit {
                name: file.name.clone(),
                enabled: Some(file.enabled),
                contents: Some(file_contents),
                ..Default::default()
            })
        })
        .collect::<Result<Vec<template::IgnitionUnit>>>()?;

    match node_image.model {
        data::NodeModel::FlatcarLinux => {
            let mut units = vec![];
            units.push(template::IgnitionUnit::mount_container_disk());
            units.extend(manifest_systemd_units);

            let container_disk = template::IgnitionFileSystem::default();

            let mut files = vec![sudo_config_file, hostname_file, disable_update];
            files.extend(manifest_text_files);

            files.push(template::IgnitionFile::ztp_interface(
                node_ipv4_address,
                mgmt_net.v4.clone(),
                node.ipv6_address,
                mgmt_net.v6.as_ref(),
            )?);

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
                vec![container_disk],
            );
            ignition_config
                .storage
                .directories
                .extend(build_ignition_home_directories(&node.text_files));
            let flatcar_config = ignition_config.to_json_pretty()?;
            let src_ztp_file = format!("{dir}/{ZTP_JSON}");
            let dst_ztp_file = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-cfg.ign");

            util::create_dir(&dir)?;
            util::create_file(&src_ztp_file, flatcar_config)?;

            // Copy blank disk for container data
            let src_data_disk = format!(
                "{}/{}/{}",
                images_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_EXT4_500MB
            );
            let dst_disk = format!("{dir}/{node_name_with_lab}-{CONTAINER_DISK_NAME}");

            util::copy_file(&src_data_disk, &dst_disk)?;

            let disk_files: Vec<&str> = manifest_binary_disk_files
                .iter()
                .map(|x| x.source.as_str())
                .collect();

            if !disk_files.is_empty() {
                util::copy_to_ext4_image(disk_files, &dst_disk, "/")?;
            }

            let dst_config_disk =
                format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-{CONTAINER_DISK_NAME}");

            Ok((src_ztp_file, dst_ztp_file, dst_disk, dst_config_disk))
        }
        _ => {
            bail!("Ignition ZTP method not supported for {}", node_image.model);
        }
    }
}

// ============================================================================
// Domain Template Building
// ============================================================================

/// Build a DomainTemplate from node data, image config, disks, interfaces, and networks.
#[instrument(
    level = "debug",
    skip(node, node_image, disks, interfaces, qemu_commands),
    fields(node_name = %node.name)
)]
#[allow(clippy::too_many_arguments)]
pub fn build_domain_template(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_id: &str,
    qemu_bin: &str,
    disks: Vec<data::NodeDisk>,
    interfaces: Vec<data::Interface>,
    qemu_commands: Vec<data::QemuCommand>,
    loopback_ipv4: String,
    management_network: String,
    isolated_network_name: String,
    reserved_network: String,
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

    Ok(template::DomainTemplate {
        qemu_bin: qemu_bin.to_string(),
        name: node_name_with_lab,
        memory,
        cpu_architecture: node_image.cpu_architecture.clone(),
        cpu_model: node_image.cpu_model.clone(),
        machine_type: node_image.machine_type.clone(),
        cpu_count,
        vmx_enabled: node_image.vmx_enabled,
        bios: node_image.bios.clone(),
        disks,
        interfaces,
        interface_type: node_image.interface_type.clone(),
        management_interface_type: management_interface_type_for_model(
            &node_image.model,
            &node_image.interface_type,
        ),
        reserved_interface_type: reserved_interface_type_for_model(
            &node_image.model,
            &node_image.interface_type,
        ),
        loopback_ipv4,
        telnet_port: TELNET_PORT,
        qemu_commands,
        lab_id: lab_id.to_string(),
        management_network,
        isolated_network: isolated_network_name,
        reserved_network,
        is_windows: matches!(
            node_image.model,
            data::NodeModel::WindowsServer | data::NodeModel::DevboxWindows
        ),
        cpu_features: cpu_features_for_model(&node_image.model),
        video_model: video_model.to_owned(),
        video_vram,
        vnc_port,
    })
}

/// Return the management interface type for a node model.
/// Some platforms (e.g. IOS-XRv9000) require e1000 for the management NIC
/// while using virtio for data interfaces.
fn management_interface_type_for_model(
    model: &data::NodeModel,
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
    model: &data::NodeModel,
    default: &data::InterfaceType,
) -> data::InterfaceType {
    match model {
        data::NodeModel::CiscoIosxrv9000 => data::InterfaceType::E1000,
        _ => default.clone(),
    }
}

/// Return CPU feature overrides for specific node models.
fn cpu_features_for_model(model: &data::NodeModel) -> Vec<data::CpuFeature> {
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

    let mut destinations = HashSet::new();
    for disk in &clone_disks {
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

    let disk_count = clone_disks.len();
    let _ = progress.send_status(
        format!("Cloning {} disks in parallel", disk_count),
        StatusKind::Progress,
    );

    let lab_id_clone = lab_id.to_string();
    let tasks: Vec<_> = clone_disks
        .into_iter()
        .map(|disk| {
            let conn: Arc<libvirt::QemuConnection> = Arc::clone(&qemu_conn);
            let progress_clone = progress.clone();
            let src = disk.src.clone();
            let dst = disk.dst.clone();
            let disk_size = disk.disk_size;
            let method = disk.method;
            let leases = image_leases.clone();
            let lab_id_task = lab_id_clone.clone();

            tokio::task::spawn(async move {
                let node_name = dst
                    .split('/')
                    .next_back()
                    .and_then(|f| f.strip_suffix("-hdd.qcow2"))
                    .unwrap_or("unknown");

                tracing::info!(
                    lab_id = %lab_id_task,
                    node_name = %node_name,
                    src = %src,
                    "Cloning disk"
                );

                let _ = progress_clone
                    .send_status(format!("Cloning disk from: {}", src), StatusKind::Progress);

                let conn_for_blocking = conn.clone();
                let src_for_blocking = src.clone();
                let dst_for_blocking = dst.clone();

                tokio::task::spawn_blocking(move || -> Result<()> {
                    let _leases = leases;
                    match method {
                        data::DiskImageCloneMethod::Overlay => libvirt::create_overlay(&conn_for_blocking, &src_for_blocking, &dst_for_blocking, disk_size)?,
                        data::DiskImageCloneMethod::Copy => {
                            libvirt::clone_disk(&conn_for_blocking, &src_for_blocking, &dst_for_blocking)
                                .with_context(|| format!("Failed to copy {src_for_blocking} to {dst_for_blocking}"))?;
                            if let Some(size_gb) = disk_size
                                && let Err(error) = libvirt::resize_disk(&conn_for_blocking, &dst_for_blocking, size_gb)
                            {
                                let name = Path::new(&dst_for_blocking).file_name().and_then(|name| name.to_str()).context("Invalid disk filename")?;
                                libvirt::delete_disk(&conn_for_blocking, name).with_context(|| format!("Resize failed ({error:#}); cannot remove newly created disk {dst_for_blocking}"))?;
                                return Err(error).context("Failed to resize copied disk");
                            }
                        }
                    }

                    Ok(())
                })
                .await
                .map_err(|e| anyhow!("Task join error: {:?}", e))??;

                let _ = progress_clone.send_status(format!("Provisioned disk to: {}", dst), StatusKind::Done);
                Ok::<String, anyhow::Error>(dst)
            })
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
        let conn = qemu_conn.clone();
        let cleanup_errors = tokio::task::spawn_blocking(move || {
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
        .context("Disk rollback task failed")?;
        failures.extend(cleanup_errors);
        bail!("Disk provisioning failed: {}", failures.join("; "));
    }

    let _ = progress.send_status(
        "All disks cloned successfully".to_string(),
        StatusKind::Done,
    );

    Ok(())
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

    let _ = progress.send_status(format!("Creating VM: {}", vm_name), StatusKind::Progress);

    let rendered_xml = domain
        .render()
        .with_context(|| format!("Failed to render XML for VM: {}", vm_name))?;

    let conn_for_blocking = qemu_conn.clone();
    let vm_name_for_blocking = vm_name.clone();

    tokio::task::spawn_blocking(move || -> Result<()> {
        libvirt::create_vm(&conn_for_blocking, &rendered_xml)
            .with_context(|| format!("Failed to create VM: {}", vm_name_for_blocking))?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("Task join error: {:?}", e))??;

    let _ = progress.send_status(format!("Created VM: {}", vm_name), StatusKind::Done);

    Ok(())
}

// ============================================================================
// Container Startup
// ============================================================================

/// Start a container node with all its network attachments.
#[allow(clippy::too_many_arguments)]
#[instrument(skip(docker, env_vars, volumes, capabilities, progress), fields(%container_name, %image), level = "debug")]
pub async fn start_container_node(
    docker: &Arc<bollard::Docker>,
    container_name: &str,
    image: &str,
    env_vars: Vec<String>,
    volumes: Vec<String>,
    capabilities: Vec<String>,
    management_network_attachment: data::ContainerNetworkAttachment,
    additional_networks: Vec<data::ContainerNetworkAttachment>,
    commands: Vec<String>,
    privileged: bool,
    shm_size: Option<i64>,
    user: Option<String>,
    model: data::NodeModel,
    progress: &ProgressSender,
) -> Result<bool> {
    let is_running = container::run_container(
        docker,
        container_name,
        image,
        env_vars,
        volumes,
        capabilities,
        management_network_attachment,
        additional_networks,
        commands,
        privileged,
        shm_size,
        user,
    )
    .await?;

    if !is_running {
        return Ok(false);
    }

    // SR Linux: flush default namespace IP to avoid DUP pings
    if model == data::NodeModel::NokiaSrlinux {
        let flush_cmd = "for i in $(seq 1 30); do ip netns list 2>/dev/null | grep -q srbase-mgmt && break; sleep 1; done; ip addr flush dev mgmt0";
        container::exec_container_detached(docker, container_name, vec!["sh", "-c", flush_cmd])
            .await
            .with_context(|| {
                format!(
                    "Failed to flush default namespace IP for {}",
                    container_name
                )
            })?;
    }

    let _ = progress.send_status(
        format!("Node {} - Started", container_name),
        StatusKind::Done,
    );

    Ok(true)
}

// ============================================================================
// Node Readiness Check
// ============================================================================

/// Check if a node is ready by attempting a TCP connection to its readiness port.
pub fn check_node_port_ready(ip: &str, port: u16) -> Result<bool> {
    let address_port = format!("{}:{}", ip, port).parse()?;
    match TcpStream::connect_timeout(&address_port, Duration::from_millis(100)) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Check if a virtual machine is ready on its readiness port. On the SSH port an SSH server
/// must answer (`check_node_ssh_ready`); any other port, such as the Talos API, needs a TCP
/// connection.
pub async fn check_vm_ready(ip: &str, port: u16) -> Result<bool> {
    if port == SSH_PORT {
        check_node_ssh_ready(ip, port).await
    } else {
        check_node_port_ready(ip, port)
    }
}

/// Check if an SSH server answers on a node: the port must accept a TCP connection and then
/// complete the SSH handshake. A port that is open before the SSH server is, such as a
/// network device's or a port forward with nothing behind it, is not ready.
///
/// No credentials are sent and nothing is trusted from the handshake, so the host key is not
/// checked. Older algorithms are allowed, as some network devices offer nothing else.
#[instrument(level = "debug")]
pub async fn check_node_ssh_ready(ip: &str, port: u16) -> Result<bool> {
    if !check_node_port_ready(ip, port)? {
        return Ok(false);
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
    match Session::probe(&hop, &options).await {
        Ok(()) => Ok(true),
        Err(error) => {
            tracing::debug!(%error, "SSH server not ready");
            Ok(false)
        }
    }
}

/// Port that shows a virtual machine is ready: the manifest's `ready_port`, else the model's
/// own port (SSH for most models, the Talos API for Talos Linux).
pub fn vm_ready_port(model: &data::NodeModel, ready_port: Option<u16>) -> u16 {
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
        .boot_mode
        .clone()
        .unwrap_or(data::UnikernelBootMode::DirectKernel);

    let mac_address = util::random_mac(KVM_OUI);
    let version = node.version.as_deref().unwrap_or(&node_image.version);
    let filename = util::image_filename(&node_image.kind, node_image.boot_mode.as_ref());

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

/// Build a unikernel domain template with auto IP injection for DirectKernel models.
#[allow(clippy::too_many_arguments)]
#[instrument(skip(node_image, disks, interfaces, mgmt_net), fields(node_name = %node.name), level = "debug")]
pub fn build_unikernel_domain_template(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    lab_id: &str,
    qemu_bin: &str,
    disks: Vec<data::NodeDisk>,
    interfaces: Vec<data::Interface>,
    kernel_path: Option<String>,
    loopback_ipv4: String,
    management_network: String,
    isolated_network_name: String,
    reserved_network: String,
    mgmt_net: &data::SherpaNetwork,
) -> template::UnikernelDomainTemplate {
    let node_name_with_lab = format!("{}-{}", node.name, lab_id);
    let boot_mode = node_image
        .boot_mode
        .clone()
        .unwrap_or(data::UnikernelBootMode::DirectKernel);

    // Determine kernel cmdline: user override > auto-injection > None
    let kernel_cmdline = if let Some(ref user_cmdline) = node.kernel_cmdline {
        Some(user_cmdline.clone())
    } else if matches!(boot_mode, data::UnikernelBootMode::DirectKernel) {
        generate_unikernel_cmdline(node, node_image, mgmt_net)
    } else {
        None
    };

    template::UnikernelDomainTemplate {
        qemu_bin: qemu_bin.to_string(),
        name: node_name_with_lab,
        memory: node.memory.unwrap_or(node_image.memory),
        cpu_architecture: node_image.cpu_architecture.clone(),
        cpu_model: node_image.cpu_model.clone(),
        machine_type: node_image.machine_type.clone(),
        cpu_count: node.cpu_count.unwrap_or(node_image.cpu_count),
        boot_mode,
        kernel_path,
        kernel_cmdline,
        disks,
        interfaces,
        interface_type: node_image.interface_type.clone(),
        management_interface_type: node_image.interface_type.clone(),
        reserved_interface_type: node_image.interface_type.clone(),
        loopback_ipv4,
        telnet_port: TELNET_PORT,
        management_network,
        isolated_network: isolated_network_name,
        reserved_network,
    }
}

/// Generate auto-injected kernel cmdline with IPv4 networking for DirectKernel unikernels.
///
/// Note: Unikraft does not support IPv6 via kernel cmdline parameters.
fn generate_unikernel_cmdline(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    mgmt_net: &data::SherpaNetwork,
) -> Option<String> {
    let _ipv4 = node.ipv4_address?;
    let _gateway_v4 = mgmt_net.v4.first;
    let _subnet_mask = mgmt_net.v4.subnet_mask;

    match node_image.model {
        // Unikraft pre-built images use DHCP for networking — cmdline IP injection
        // is not supported (the ELF loader treats cmdline as application path).
        // Static DHCP binding (MAC -> IP) on the sherpa router handles IP assignment.
        data::NodeModel::UnikraftUnikernel => None,
        _ => None,
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
        format!("Creating unikernel: {}", uk_name),
        StatusKind::Progress,
    );

    let rendered_xml = domain
        .render()
        .with_context(|| format!("Failed to render XML for unikernel: {}", uk_name))?;

    let conn_for_blocking = qemu_conn.clone();
    let uk_name_for_blocking = uk_name.clone();

    tokio::task::spawn_blocking(move || -> Result<()> {
        libvirt::create_vm(&conn_for_blocking, &rendered_xml)
            .with_context(|| format!("Failed to create unikernel: {}", uk_name_for_blocking))?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("Task join error: {:?}", e))??;

    let _ = progress.send_status(format!("Created unikernel: {}", uk_name), StatusKind::Done);

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
    let domain = match virt::domain::Domain::lookup_by_name(qemu_conn, domain_name) {
        Ok(d) => d,
        Err(_) => return Ok(false),
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
    let node_name_with_lab = format!("{}-{}", node_name, lab_id);

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
    .map_err(|e| anyhow!("Task join error: {:?}", e))??;

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
    .map_err(|e| anyhow!("Task join error: {:?}", e))??;

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
    .map_err(|e| anyhow!("Task join error: {:?}", e))??;

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
        .map_err(|e| anyhow!("Task join error: {:?}", e))??;
    }

    Ok(())
}

/// Destroy a single container node: kill, remove, and delete associated Docker networks.
#[instrument(skip(docker), fields(%node_name, %lab_id), level = "debug")]
pub async fn destroy_container_node(
    docker: &bollard::Docker,
    node_name: &str,
    lab_id: &str,
) -> Result<()> {
    let container_name = format!("{}-{}", node_name, lab_id);
    let _ = container::kill_container(docker, &container_name).await;
    let _ = container::remove_container(docker, &container_name).await;

    // Destroy Docker networks belonging to this node
    if let Ok(networks) = container::list_networks(docker).await {
        for net in networks {
            if let Some(net_name) = net.name
                && net_name.starts_with(&format!("{}-", node_name))
                && net_name.ends_with(&format!("-{}", lab_id))
            {
                let _ = container::delete_network(docker, &net_name).await;
                tracing::info!(network = %net_name, "Destroyed Docker network for node");
            }
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
        assert!(runcmd.is_empty());
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
        assert_eq!(env_file.mode, 600);
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
            cloud_init_ipv6_address(&data::NodeModel::OpenBsd, Some(address)),
            None
        );
        assert_eq!(
            cloud_init_ipv6_address(&data::NodeModel::FreeBsd, Some(address)),
            Some(address)
        );
        assert_eq!(
            cloud_init_ipv6_address(&data::NodeModel::UbuntuLinux, Some(address)),
            Some(address)
        );
    }

    #[test]
    fn test_openbsd_wheel_gets_passwordless_doas() {
        let file = cloud_init_doas_file(&data::NodeModel::OpenBsd).unwrap();
        assert_eq!(file.path, "/etc/doas.conf");
        assert_eq!(file.content, "permit nopass keepenv :wheel\n");
        assert_eq!(file.permissions, "0400");
        // Root-owned files leave the owner out, so BSD uses root:wheel.
        assert_eq!(file.owner, None);
        assert!(cloud_init_doas_file(&data::NodeModel::FreeBsd).is_none());
        assert!(cloud_init_doas_file(&data::NodeModel::UbuntuLinux).is_none());
    }

    #[test]
    fn test_openbsd_logins_get_no_sudo_rule() {
        assert_eq!(cloud_init_sudo_rule(&data::NodeModel::OpenBsd), None);
        for model in [data::NodeModel::FreeBsd, data::NodeModel::UbuntuLinux] {
            assert_eq!(
                cloud_init_sudo_rule(&model).as_deref(),
                Some("ALL=(ALL) NOPASSWD:ALL"),
                "{model}"
            );
        }
    }

    #[test]
    fn test_openbsd_cloud_init_does_not_manage_etc_hosts() {
        assert!(!cloud_init_manage_etc_hosts(&data::NodeModel::OpenBsd));
        assert!(cloud_init_manage_etc_hosts(&data::NodeModel::FreeBsd));
        assert!(cloud_init_manage_etc_hosts(&data::NodeModel::UbuntuLinux));
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
                r#"'/opt/sherpa/startup_scripts/boot.sh'"#.to_string(),
                r#"chown -R 'alice':"$(id -gn 'alice')" '/opt/sherpa/user_scripts'"#.to_string(),
                r#"su - 'alice' -c ''\''/opt/sherpa/user_scripts/user.sh'\'''"#.to_string(),
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
            vec![r#"'/opt/sherpa/startup_scripts/boot.sh'"#.to_string()]
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
        let dirs = build_ignition_home_directories(&text_files);
        let mut paths = Vec::new();
        for dir in &dirs {
            assert_eq!(dir.user.as_ref().unwrap().name, "alice");
            assert_eq!(dir.group.as_ref().unwrap().name, "alice");
            assert_eq!(dir.mode, 755);
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
            "powershell -Command \"[System.Environment]::SetEnvironmentVariable('EDITOR', 'vim', 'Machine')\""
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
        assert!(commands.is_empty());
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

        let result = build_cloud_init_text_files(&text_files);
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
        let result = build_cloud_init_text_files(&None);
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

        let result = build_cloud_init_text_files(&text_files);
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

        let result = build_cloudbase_text_files(&text_files);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, r"C:\app\config.json");
        assert_eq!(result[0].content, "aGVsbG8=");
        assert_eq!(result[0].permissions, "0644");
    }

    #[test]
    fn test_build_cloudbase_text_files_none() {
        let result = build_cloudbase_text_files(&None);
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
        assert!(build_windows_private_file_acls(&files).unwrap().is_empty());
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
            cloud_init_user_settings(&data::NodeModel::OmarchyLinux, &data::OsVariant::Linux),
            ("wheel", "/bin/bash")
        );
        assert_eq!(
            cloud_init_user_settings(&data::NodeModel::UbuntuLinux, &data::OsVariant::Linux),
            ("sudo", "/bin/bash")
        );
        assert_eq!(
            cloud_init_user_settings(&data::NodeModel::FreeBsd, &data::OsVariant::Bsd),
            ("wheel", "/bin/sh")
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
            vec![],
            vec![],
            vec![],
            "127.0.0.1".to_owned(),
            "mgmt".to_owned(),
            String::new(),
            String::new(),
            &data::NodeResources {
                cpu_count: Some(2),
                memory: Some(4096),
                vnc_port: Some(5900),
            },
        )
        .unwrap();
        let xml = domain.render().unwrap();
        assert_eq!(domain.video_model, "virtio");
        assert!(domain.video_vram.is_none());
        assert!(xml.contains("<model type='virtio' heads='1'/>"));
        assert!(xml.contains("OVMF_CODE_4M.fd"));
        assert!(xml.find("<boot dev='cdrom'/>").unwrap() < xml.find("<boot dev='hd'/>").unwrap());
        assert_eq!(config.ztp_method, data::ZtpMethod::CloudInit);
        assert!(config.cdrom.is_none());
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
        let user = data::User {
            username: SHERPA_USERNAME.to_owned(),
            password: None,
            ssh_public_key: data::SshPublicKey {
                algorithm: data::SshKeyAlgorithms::SshEd25519,
                key: "legacy-key-fixture".to_owned(),
                comment: None,
            },
            sudo: true,
        };
        let account = GuestAccount {
            username: "alice".to_owned(),
            ssh_keys: vec!["ssh-ed25519 personal-key-fixture".to_owned()],
        };
        let network = data::SherpaNetwork {
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
        };
        let (tx, _rx) = mpsc::unbounded_channel();
        let progress = ProgressSender::new(tx);
        for personal in [None, Some(&account)] {
            let (config_path, _, _, _) = generate_ignition_ztp(
                &node,
                &image,
                root,
                root,
                &user,
                &network,
                "192.0.2.10".parse().unwrap(),
                "vm-test",
                &progress,
                None,
                personal,
            )
            .unwrap();
            let json: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(config_path).unwrap()).unwrap();
            let files = json["storage"]["files"].as_array().unwrap();
            let legacy = files
                .iter()
                .find(|file| file["path"] == format!("/etc/sudoers.d/{SHERPA_USERNAME}"))
                .unwrap();
            assert_eq!(legacy["mode"], 0o440);
            let legacy_rule = util::base64_decode(
                legacy["contents"]["source"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("data:;base64,")
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                legacy_rule,
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
                let rule = util::base64_decode(
                    personal_file["contents"]["source"]
                        .as_str()
                        .unwrap()
                        .strip_prefix("data:;base64,")
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(rule, "alice ALL=(ALL) NOPASSWD: ALL\n");
                assert_eq!(json["passwd"]["users"][1]["name"], "alice");
                assert!(json["passwd"]["users"][1].get("passwordHash").is_none());
            } else {
                assert!(personal_file.is_none());
                assert_eq!(json["passwd"]["users"].as_array().unwrap().len(), 1);
            }
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
            vm_ready_port(&data::NodeModel::TalosLinux, None),
            TALOS_API_PORT
        );
        assert_eq!(vm_ready_port(&data::NodeModel::UbuntuLinux, None), SSH_PORT);
        assert_eq!(
            vm_ready_port(&data::NodeModel::UbuntuLinux, Some(8443)),
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
        assert!(!check_node_ssh_ready("127.0.0.1", port).await.unwrap());
        task.abort();
    }

    #[tokio::test]
    async fn test_check_node_ssh_ready_false_when_closed() {
        assert!(!check_node_ssh_ready("127.0.0.1", 1).await.unwrap());
    }

    #[tokio::test]
    async fn test_check_vm_ready_uses_tcp_for_other_ports() {
        let (port, task) = listener_replying(b"not ssh\r\n").await;
        assert!(check_vm_ready("127.0.0.1", port).await.unwrap());
        task.abort();
    }
}
