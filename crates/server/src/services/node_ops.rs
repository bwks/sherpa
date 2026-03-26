// Shared per-node operations extracted from up.rs for reuse by redeploy.rs

use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use askama::Template;

use crate::services::progress::ProgressSender;

use shared::data;
use shared::data::StatusKind;
use shared::konst::{
    ARISTA_ABOOT_DIR, ARISTA_CEOS_ZTP_VOLUME_MOUNT, CISCO_ASAV_ZTP_CONFIG, CISCO_FTDV_ZTP_CONFIG,
    CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_ZTP_CONFIG, CISCO_IOSXR_ZTP_CONFIG, CISCO_ISE_ZTP_CONFIG,
    CISCO_NXOS_ZTP_CONFIG, CLOUD_INIT_META_DATA, CLOUD_INIT_NETWORK_CONFIG, CLOUD_INIT_USER_DATA,
    CONTAINER_ARISTA_CEOS_COMMANDS, CONTAINER_ARISTA_CEOS_ENV_VARS, CONTAINER_ARISTA_CEOS_REPO,
    CONTAINER_DISK_NAME, CONTAINER_FRR_ENV_VARS, CONTAINER_FRR_REPO, CONTAINER_GITLAB_CE_COMMANDS,
    CONTAINER_GITLAB_CE_REPO, CONTAINER_GITLAB_CE_SHM_SIZE, CONTAINER_HASHICORP_VAULT_CAPABILITIES,
    CONTAINER_HASHICORP_VAULT_COMMANDS, CONTAINER_HASHICORP_VAULT_ENV_VARS,
    CONTAINER_HASHICORP_VAULT_REPO, CONTAINER_NOKIA_SRLINUX_COMMANDS,
    CONTAINER_NOKIA_SRLINUX_ENV_VARS, CONTAINER_NOKIA_SRLINUX_REPO, CONTAINER_NOKIA_SRLINUX_USER,
    CONTAINER_SURREAL_DB_COMMANDS, CONTAINER_SURREAL_DB_REPO, CUMULUS_ZTP, FRR_ZTP_CONFIG_MOUNT,
    FRR_ZTP_DAEMONS_MOUNT, FRR_ZTP_STARTUP_MOUNT, JUNIPER_ZTP_CONFIG, JUNIPER_ZTP_CONFIG_TGZ,
    KVM_OUI, MIKROTIK_CHR_ZTP_CONFIG, NODE_CERTS_DIR, NODE_CERTS_DIR_WINDOWS, NODE_CONFIGS_DIR,
    NODE_USER_SCRIPTS_DIR, NOKIA_SRLINUX_ZTP_VOLUME_MOUNT, PALOALTO_BOOTSTRAP_CONFIG,
    PALOALTO_ZTP_CONFIG, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_EXT4_500MB,
    SHERPA_BLANK_DISK_FAT32, SHERPA_BLANK_DISK_IOSV, SHERPA_BLANK_DISK_ISE,
    SHERPA_BLANK_DISK_JUNOS, SHERPA_DOMAIN_NAME, SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX,
    SHERPA_ISOLATED_NETWORK_NAME, SHERPA_PASSWORD, SHERPA_PASSWORD_HASH,
    SHERPA_PASSWORD_HASH_SHA256, SHERPA_RESERVED_NETWORK_BRIDGE_PREFIX,
    SHERPA_RESERVED_NETWORK_NAME, SHERPA_SSH_PUBLIC_KEY_PATH, SHERPA_STORAGE_POOL_PATH,
    SHERPA_USERNAME, SSH_PORT, TELNET_PORT, VAULT_ZTP_CONFIG_MOUNT, ZTP_DIR, ZTP_ISO, ZTP_JSON,
};
use shared::util;
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
        config_file: format!("{}.conf", &node.name),
        ipv4_address: node_ipv4_address,
        ipv6_address: None,
        mac_address: String::new(),
        ztp_method: node_image.ztp_method.clone(),
        ssh_port: SSH_PORT,
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
pub fn generate_vm_ztp(
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
) -> Result<VmZtpResult> {
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
    let src_boot_disk = format!(
        "{}/{}/{}/virtioa.qcow2",
        images_dir, node_image.model, node_image.version
    );
    let dst_boot_disk = format!("{SHERPA_STORAGE_POOL_PATH}/{node_name_with_lab}-hdd.qcow2");

    clone_disks.push(data::CloneDisk {
        src: src_boot_disk,
        dst: dst_boot_disk.clone(),
        disk_size: node.boot_disk_size,
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
                )?;
                src_ignition_disk = Some(src_ign);
                dst_ignition_disk = Some(dst_ign);
                src_config_disk = Some(src_cfg);
                dst_config_disk = Some(dst_cfg);
            }
            _ => {
                let _ = progress.send_status(
                    format!(
                        "ZTP method {:?} not yet implemented for VM: {}",
                        node_image.ztp_method, node.name
                    ),
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
        });
        disks.push(data::NodeDisk {
            disk_device: data::DiskDevices::File,
            driver_name: data::DiskDrivers::Qemu,
            driver_format: data::DiskFormats::Raw,
            src_file: dst_disk,
            target_dev: data::DiskTargets::target(&data::DiskBuses::Usb, 0)?,
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
        config_file: format!("{}.conf", &node.name),
        ipv4_address: node_ipv4_address,
        ipv6_address: None,
        mac_address: mac_address.to_string(),
        ztp_method: node_image.ztp_method.clone(),
        ssh_port: SSH_PORT,
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
            owner: Some("root:root".to_string()),
            encoding: None,
        },
        template::CloudInitWriteFile {
            path: format!("{target_dir}/node.crt"),
            content: node_cert_content,
            permissions: "0644".to_string(),
            owner: Some("root:root".to_string()),
            encoding: None,
        },
        template::CloudInitWriteFile {
            path: format!("{target_dir}/node.key"),
            content: node_key_content,
            permissions: "0600".to_string(),
            owner: Some("root:root".to_string()),
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
            mode: 0o644,
            contents: template::IgnitionFileContents::new(&format!("data:;base64,{ca_b64}")),
            ..Default::default()
        },
        template::IgnitionFile {
            path: format!("{target_dir}/node.crt"),
            mode: 0o644,
            contents: template::IgnitionFileContents::new(&format!("data:;base64,{cert_b64}")),
            ..Default::default()
        },
        template::IgnitionFile {
            path: format!("{target_dir}/node.key"),
            mode: 0o600,
            contents: template::IgnitionFileContents::new(&format!("data:;base64,{key_b64}")),
            ..Default::default()
        },
    ])
}

// ============================================================================
// Environment variable helpers
// ============================================================================

/// Build a shell script for `/etc/profile.d/sherpa-env.sh` that exports
/// environment variables for login shells.
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

/// Build entries for `/etc/environment` (KEY=VALUE format, no `export`).
/// Values containing spaces or special characters are double-quoted.
fn build_etc_environment_entries(env_vars: &[String]) -> String {
    let mut entries = String::new();
    for entry in env_vars {
        if let Some((key, value)) = entry.split_once('=') {
            if value.contains(' ')
                || value.contains('\'')
                || value.contains('"')
                || value.contains('\\')
            {
                let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
                entries.push_str(&format!("{}=\"{}\"\n", key, escaped));
            } else {
                entries.push_str(&format!("{}={}\n", key, value));
            }
        }
    }
    entries
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

// ============================================================================
// VM ZTP sub-methods
// ============================================================================

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
        | data::NodeModel::JenkinsServer
        | data::NodeModel::NautobotServer
        | data::NodeModel::NetboxServer
        | data::NodeModel::VirtServer
        | data::NodeModel::FreeBsd
        | data::NodeModel::OpenBsd => {
            let (admin_group, shell) = match node_image.os_variant {
                data::OsVariant::Bsd => ("wheel".to_string(), "/bin/sh".to_string()),
                _ => ("sudo".to_string(), "/bin/bash".to_string()),
            };
            cloud_init_user.groups = vec![admin_group];
            cloud_init_user.shell = shell;

            let mut write_files = match cert_paths {
                Some(certs) => build_cloud_init_cert_files(certs, NODE_CERTS_DIR)?,
                None => vec![],
            };

            let mut runcmd: Vec<String> = vec![];

            // Inject startup_scripts into write_files and runcmd
            if let Some(ref scripts) = node.startup_scripts {
                for script in scripts {
                    let contents = util::base64_decode(&script.content).with_context(|| {
                        format!(
                            "Failed to decode startup_script '{}' for node '{}'",
                            script.filename, node.name
                        )
                    })?;

                    let target_path = format!("/opt/sherpa/startup_scripts/{}", script.filename);

                    write_files.push(template::CloudInitWriteFile {
                        path: target_path.clone(),
                        content: contents,
                        permissions: "0755".to_string(),
                        owner: Some("root:root".to_string()),
                        encoding: None,
                    });

                    runcmd.push(format!("bash {}", target_path));
                }
            }

            // Inject user_scripts into write_files and runcmd (run as sherpa user).
            // Files are written as root (write_files runs before users are created)
            // then chowned via runcmd after the user exists.
            if let Some(ref scripts) = node.user_scripts {
                let user = SHERPA_USERNAME;
                for script in scripts {
                    let contents = util::base64_decode(&script.content).with_context(|| {
                        format!(
                            "Failed to decode user_script '{}' for node '{}'",
                            script.filename, node.name
                        )
                    })?;

                    let target_path = format!("{}/{}", NODE_USER_SCRIPTS_DIR, script.filename);

                    write_files.push(template::CloudInitWriteFile {
                        path: target_path.clone(),
                        content: contents,
                        permissions: "0755".to_string(),
                        owner: None,
                        encoding: None,
                    });
                }

                runcmd.push(format!(
                    "chown -R {}:{} {}",
                    user, user, NODE_USER_SCRIPTS_DIR
                ));
                for script in scripts {
                    let target_path = format!("{}/{}", NODE_USER_SCRIPTS_DIR, script.filename);
                    runcmd.push(format!("su - {} -c 'bash {}'", user, target_path));
                }
            }

            // Inject environment_variables into profile.d and /etc/environment
            if let Some(ref env_vars) = node.environment_variables
                && !env_vars.is_empty()
            {
                write_files.push(template::CloudInitWriteFile {
                    path: "/etc/profile.d/sherpa-env.sh".to_string(),
                    content: build_env_profile_script(env_vars),
                    permissions: "0644".to_string(),
                    owner: Some("root:root".to_string()),
                    encoding: None,
                });

                let etc_env_content = build_etc_environment_entries(env_vars);
                runcmd.push(format!(
                    "printf '{}' >> /etc/environment",
                    etc_env_content.replace('\'', "'\\''")
                ));
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
                manage_etc_hosts: true,
                ssh_pwauth: true,
                users: vec![cloud_init_user],
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
                node.ipv6_address,
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

            // Inject startup_scripts for Windows
            let mut runcmd = vec![
                format!(
                    "icacls \"{}\" /inheritance:r /grant \"Administrators:F\" /grant \"SYSTEM:F\"",
                    admin_keys_path
                ),
                "powershell -Command \"Restart-Service sshd\"".to_string(),
            ];

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

            let mut runcmd: Vec<String> = vec![];

            // Inject startup_scripts into write_files and runcmd
            if let Some(ref scripts) = node.startup_scripts {
                for script in scripts {
                    let contents = util::base64_decode(&script.content).with_context(|| {
                        format!(
                            "Failed to decode startup_script '{}' for node '{}'",
                            script.filename, node.name
                        )
                    })?;

                    let target_path = format!("/opt/sherpa/startup_scripts/{}", script.filename);

                    write_files.push(template::CloudInitWriteFile {
                        path: target_path.clone(),
                        content: contents,
                        permissions: "0755".to_string(),
                        owner: Some("root:root".to_string()),
                        encoding: None,
                    });

                    runcmd.push(format!("bash {}", target_path));
                }
            }

            // Inject user_scripts into write_files and runcmd (run as sherpa user).
            // Files are written as root (write_files runs before users are created)
            // then chowned via runcmd after the user exists.
            if let Some(ref scripts) = node.user_scripts {
                let user = SHERPA_USERNAME;
                for script in scripts {
                    let contents = util::base64_decode(&script.content).with_context(|| {
                        format!(
                            "Failed to decode user_script '{}' for node '{}'",
                            script.filename, node.name
                        )
                    })?;

                    let target_path = format!("{}/{}", NODE_USER_SCRIPTS_DIR, script.filename);

                    write_files.push(template::CloudInitWriteFile {
                        path: target_path.clone(),
                        content: contents,
                        permissions: "0755".to_string(),
                        owner: None,
                        encoding: None,
                    });
                }

                runcmd.push(format!(
                    "chown -R {}:{} {}",
                    user, user, NODE_USER_SCRIPTS_DIR
                ));
                for script in scripts {
                    let target_path = format!("{}/{}", NODE_USER_SCRIPTS_DIR, script.filename);
                    runcmd.push(format!("su - {} -c 'bash {}'", user, target_path));
                }
            }

            // Inject environment_variables into profile.d and /etc/environment
            if let Some(ref env_vars) = node.environment_variables
                && !env_vars.is_empty()
            {
                write_files.push(template::CloudInitWriteFile {
                    path: "/etc/profile.d/sherpa-env.sh".to_string(),
                    content: build_env_profile_script(env_vars),
                    permissions: "0644".to_string(),
                    owner: Some("root:root".to_string()),
                    encoding: None,
                });

                let etc_env_content = build_etc_environment_entries(env_vars);
                runcmd.push(format!(
                    "printf '{}' >> /etc/environment",
                    etc_env_content.replace('\'', "'\\''")
                ));
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
                manage_etc_hosts: true,
                ssh_pwauth: true,
                users: vec![cloud_init_user],
                write_files: cloud_init_write_files,
                runcmd: cloud_init_runcmd,
                ..Default::default()
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

            let ztp_init = format!("{dir}/{}.conf", &node.name);
            let sonic_ztp = template::SonicLinuxZtp {
                hostname: node.name.clone(),
                mgmt_ipv4: mgmt_net.v4.clone(),
                mgmt_ipv4_address: Some(node_ipv4_address),
                mgmt_ipv6_address: node.ipv6_address,
                mgmt_ipv6: mgmt_net.v6.clone(),
            };
            let ztp_config = format!("{dir}/{}_config_db.json", &node.name);
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
        password_hash: SHERPA_PASSWORD_HASH.to_owned(),
        ssh_authorized_keys: authorized_keys,
        groups: vec!["wheel".to_owned(), "docker".to_owned()],
    };

    let hostname_file = template::IgnitionFile {
        path: "/etc/hostname".to_owned(),
        mode: 644,
        contents: template::IgnitionFileContents::new(&format!("data:,{dev_name}")),
        ..Default::default()
    };

    let disable_update = template::IgnitionFile::disable_updates();
    let sudo_config_base64 =
        util::base64_encode(&format!("{SHERPA_USERNAME} ALL=(ALL) NOPASSWD: ALL"));
    let sudo_config_file = template::IgnitionFile {
        path: format!("/etc/sudoers.d/{SHERPA_USERNAME}"),
        mode: 440,
        contents: template::IgnitionFileContents::new(&format!(
            "data:;base64,{sudo_config_base64}"
        )),
        ..Default::default()
    };

    let manifest_text_files: Vec<template::IgnitionFile> = node
        .text_files
        .iter()
        .flatten()
        .map(|file| {
            let encoded_file = util::base64_encode_file(&file.source)?;

            Ok(template::IgnitionFile {
                path: file.destination.clone(),
                mode: file.permissions,
                overwrite: None,
                contents: template::IgnitionFileContents::new(&format!(
                    "data:;base64,{encoded_file}"
                )),
                user: Some(template::IgnitionFileParams {
                    name: file.user.clone(),
                }),
                group: Some(template::IgnitionFileParams {
                    name: file.group.clone(),
                }),
            })
        })
        .collect::<Result<Vec<template::IgnitionFile>>>()?;

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

            // Inject environment_variables via profile.d and /etc/environment
            if let Some(ref env_vars) = node.environment_variables
                && !env_vars.is_empty()
            {
                let profile_script = build_env_profile_script(env_vars);
                let profile_b64 = util::base64_encode(&profile_script);
                files.push(template::IgnitionFile {
                    path: "/etc/profile.d/sherpa-env.sh".to_string(),
                    mode: 0o644,
                    contents: template::IgnitionFileContents::new(&format!(
                        "data:;base64,{profile_b64}"
                    )),
                    ..Default::default()
                });

                let etc_env = build_etc_environment_entries(env_vars);
                let etc_env_b64 = util::base64_encode(&etc_env);
                files.push(template::IgnitionFile {
                    path: "/etc/environment".to_string(),
                    mode: 0o644,
                    overwrite: Some(true),
                    contents: template::IgnitionFileContents::new(&format!(
                        "data:;base64,{etc_env_b64}"
                    )),
                    ..Default::default()
                });
            }

            let ignition_config = template::IgnitionConfig::new(
                vec![ignition_user],
                files,
                vec![],
                units,
                vec![],
                vec![container_disk],
            );
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
) -> template::DomainTemplate {
    let node_name_with_lab = format!("{}-{}", node.name, lab_id);

    template::DomainTemplate {
        qemu_bin: qemu_bin.to_string(),
        name: node_name_with_lab,
        memory: node.memory.unwrap_or(node_image.memory),
        cpu_architecture: node_image.cpu_architecture.clone(),
        cpu_model: node_image.cpu_model.clone(),
        machine_type: node_image.machine_type.clone(),
        cpu_count: node.cpu_count.unwrap_or(node_image.cpu_count),
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
    }
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
pub async fn clone_node_disks(
    qemu_conn: Arc<libvirt::QemuConnection>,
    clone_disks: Vec<data::CloneDisk>,
    lab_id: &str,
    progress: &ProgressSender,
) -> Result<()> {
    if clone_disks.is_empty() {
        let _ = progress.send_status("No disks to clone".to_string(), StatusKind::Info);
        return Ok(());
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
                    libvirt::clone_disk(&conn_for_blocking, &src_for_blocking, &dst_for_blocking)
                        .with_context(|| {
                        format!(
                            "Failed to clone disk from: {} to: {}",
                            src_for_blocking, dst_for_blocking
                        )
                    })?;

                    if let Some(size_gb) = disk_size {
                        libvirt::resize_disk(&conn_for_blocking, &dst_for_blocking, size_gb)
                            .with_context(|| {
                                format!(
                                    "Failed to resize disk '{}' to {}G",
                                    dst_for_blocking, size_gb
                                )
                            })?;
                    }

                    Ok(())
                })
                .await
                .map_err(|e| anyhow!("Task join error: {:?}", e))??;

                progress_clone.send_status(format!("Cloned disk to: {}", dst), StatusKind::Done)?;
                Ok::<(), anyhow::Error>(())
            })
        })
        .collect();

    for task in tasks {
        task.await.context("Disk cloning task failed")??;
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

/// Check if a node is ready by attempting a TCP connection to its SSH port.
pub fn check_node_ready_ssh(ip: &str, port: u16) -> Result<bool> {
    validate::tcp_connect(ip, port)
}

// ============================================================================
// Database Connection
// ============================================================================

/// Connect to the SurrealDB database using environment variables for credentials.
pub async fn connect_db()
-> Result<std::sync::Arc<surrealdb::Surreal<surrealdb::engine::remote::ws::Client>>> {
    use shared::konst::{
        SHERPA_DB_NAME, SHERPA_DB_NAMESPACE, SHERPA_DB_PORT, SHERPA_DB_SERVER, SHERPA_ENV_FILE_PATH,
    };

    let db_password = std::env::var("SHERPA_DB_PASSWORD").context(format!(
        "SHERPA_DB_PASSWORD environment variable is not set (check {})",
        SHERPA_ENV_FILE_PATH
    ))?;

    let db_port = std::env::var("SHERPA_DB_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(SHERPA_DB_PORT);

    db::connect(
        SHERPA_DB_SERVER,
        db_port,
        SHERPA_DB_NAMESPACE,
        SHERPA_DB_NAME,
        &db_password,
    )
    .await
    .context("Failed to connect to database")
}

// ============================================================================
// Node Destruction
// ============================================================================

/// Destroy a single VM node: destroy domain, undefine, delete disks, and destroy networks.
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
        if let Ok(domain) = virt::domain::Domain::lookup_by_name(&conn, &vm_name) {
            if domain.is_active()? {
                domain.destroy()?;
            }
            domain.undefine_flags(VIR_DOMAIN_UNDEFINE_NVRAM)?;
            tracing::info!(vm_name = %vm_name, "VM destroyed and undefined");
        }
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("Task join error: {:?}", e))??;

    // Delete VM disks from storage pool
    let conn = qemu_conn.clone();
    let disk_prefix = node_name_with_lab;
    tokio::task::spawn_blocking(move || -> Result<()> {
        if let Ok(pool) = virt::storage_pool::StoragePool::lookup_by_name(&conn, "sherpa-pool")
            && let Ok(volumes) = pool.list_all_volumes(0)
        {
            for vol in volumes {
                if let Ok(vol_name) = vol.get_name()
                    && vol_name.starts_with(&disk_prefix)
                {
                    tracing::info!(volume = %vol_name, "Deleting disk");
                    let _ = vol.delete(0);
                }
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
    fn test_build_etc_environment_entries_simple() {
        let env_vars = vec!["EDITOR=vim".to_string(), "TOKEN=abc123".to_string()];
        let entries = build_etc_environment_entries(&env_vars);
        assert_eq!(entries, "EDITOR=vim\nTOKEN=abc123\n");
    }

    #[test]
    fn test_build_etc_environment_entries_with_spaces() {
        let env_vars = vec!["MSG=hello world".to_string()];
        let entries = build_etc_environment_entries(&env_vars);
        assert_eq!(entries, "MSG=\"hello world\"\n");
    }

    #[test]
    fn test_build_etc_environment_entries_with_quotes() {
        let env_vars = vec!["MSG=say \"hi\"".to_string()];
        let entries = build_etc_environment_entries(&env_vars);
        assert_eq!(entries, "MSG=\"say \\\"hi\\\"\"\n");
    }

    #[test]
    fn test_build_etc_environment_entries_empty() {
        let env_vars: Vec<String> = vec![];
        let entries = build_etc_environment_entries(&env_vars);
        assert_eq!(entries, "");
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
}
