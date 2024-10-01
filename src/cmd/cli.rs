use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use askama::Template;

use clap::{Parser, Subcommand};
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;
use virt::sys;

use crate::core::konst::{
    ARISTA_OUI, BOOT_NETWORK_BRIDGE, BOOT_NETWORK_DHCP_END, BOOT_NETWORK_DHCP_START,
    BOOT_NETWORK_HTTP_SERVER, BOOT_NETWORK_IP, BOOT_NETWORK_NAME, BOOT_NETWORK_NETMASK, BOXES_DIR,
    CISCO_IOSXE_ZTP_CONFIG, CISCO_OUI, CLOUD_INIT_META_DATA, CLOUD_INIT_USER_DATA, CONFIG_DIR,
    CONFIG_FILE, ISOLATED_NETWORK_BRIDGE, ISOLATED_NETWORK_NAME, JUNIPER_OUI, KVM_OUI,
    MANIFEST_FILE, READINESS_SLEEP, READINESS_TIMEOUT, SHERPA_SSH_PRIVATE_KEY_FILE,
    SHERPA_USERNAME, SSH_PORT, STORAGE_POOL, STORAGE_POOL_PATH, TELNET_PORT, TEMP_DIR, ZTP_ISO,
};
use crate::core::{Config, Sherpa};
use crate::libvirt::{
    clone_disk, create_isolated_network, create_network, create_vm, delete_disk, get_mgmt_ip,
    CiscoIosXeZtpTemplate, CloudInitTemplate, DomainTemplate, Qemu,
};
use crate::model::{ConnectionTypes, DeviceModels, Interface, OsVariants, User, ZtpMethods};
use crate::topology::{ConnectionMap, Manifest};
use crate::util::{
    copy_file, create_dir, create_file, create_ztp_iso, dir_exists, file_exists,
    fix_permissions_recursive, generate_ssh_keypair, get_ip, id_to_port, pub_ssh_key_to_md5_hash,
    random_mac, tcp_connect, term_msg_surround, term_msg_underline,
};

// Used to clone disk for VM creation
struct CloneDisk {
    src: String,
    dst: String,
}

struct _ZtpDisk {
    name: String,
    files: Vec<String>,
}

#[derive(Default, Debug, Parser)]
#[command(name = "sherpa")]
#[command(bin_name = "sherpa")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Sherpa - Network Lab Management", long_about = None)]
pub struct Cli {
    #[clap(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Initialise a Sherpa environment
    Init {
        /// Name of the config file
        #[arg(default_value = CONFIG_FILE)]
        config_file: String,

        /// Name of the manifest file
        #[arg(default_value = MANIFEST_FILE)]
        manifest_file: String,

        /// Overwrite config file if one exists
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    /// Build environment
    Up {
        /// Name of the config file
        #[arg(default_value = CONFIG_FILE)]
        config_file: String,
    },
    /// Stop environment
    Down,
    /// Resume environment
    Resume,
    /// Destroy environment
    Destroy,
    /// Inspect environment
    Inspect,

    /// Fix up environment
    Doctor {
        /// Set base box permissions to read-only
        #[arg(long, action = clap::ArgAction::SetTrue)]
        boxes: bool,
    },

    /// Clean up environment
    Cleaner {
        /// Remove all devices, disks and networks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        all: bool,
        /// Remove all disks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        disks: bool,
        /// Remove all networks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        networks: bool,
    },

    /// Import a disk image
    Import {
        /// Source path of the disk image
        #[arg(short, long)]
        src: String,
        /// Version of the device model
        #[arg(short, long)]
        version: String,
        /// Model of Device
        #[arg(short, long, value_enum)]
        model: DeviceModels,
        /// Import the disk image as the latest version
        #[arg(long, action = clap::ArgAction::SetTrue)]
        latest: bool,
    },

    /// Connect to a device via serial console over Telnet
    Console { name: String },

    /// SSH to a device.
    Ssh { name: String },
}
impl Default for Commands {
    fn default() -> Self {
        Commands::Init {
            config_file: CONFIG_FILE.to_owned(),
            manifest_file: MANIFEST_FILE.to_owned(),
            force: false,
        }
    }
}

impl Cli {
    pub fn run() -> Result<()> {
        let cli = Cli::parse();
        let qemu = Qemu::default();
        let mut sherpa = Sherpa::default();

        match &cli.commands {
            Commands::Init {
                config_file,
                manifest_file,
                force,
            } => {
                term_msg_surround("Sherpa Initializing");
                let qemu_conn = qemu.connect()?;

                sherpa.config_path = format!("{}/{}", sherpa.config_dir, config_file);

                println!("- Creating Files -");
                // Create the default config directories
                if dir_exists(&sherpa.config_dir) && !*force {
                    println!("Directory path already exists: {}", sherpa.config_dir);
                } else {
                    create_dir(&sherpa.config_dir)?;
                    create_dir(&sherpa.boxes_dir)?;
                    // box directories
                    let config = Config::default();
                    for device_model in config.device_models {
                        create_dir(&format!(
                            "{}/{}/latest",
                            sherpa.boxes_dir, device_model.name
                        ))?;
                    }
                }

                // Initialize default files
                if file_exists(&sherpa.config_path) && !*force {
                    println!("Config file already exists: {}", sherpa.config_path);
                } else {
                    let config = Config {
                        name: config_file.to_owned(),
                        ..Default::default()
                    };
                    config.create(&sherpa.config_path)?;
                }

                if file_exists(manifest_file) && !*force {
                    println!("Manifest file already exists: {manifest_file}");
                } else {
                    let manifest = Manifest::default();
                    manifest.write_file()?;
                }

                println!("- Creating Networks -");
                // Initialize the sherpa boot network
                if qemu_conn
                    .list_networks()?
                    .iter()
                    .any(|net| net == BOOT_NETWORK_NAME)
                {
                    println!("Network already exists: {BOOT_NETWORK_NAME}");
                } else {
                    println!("Creating network: {BOOT_NETWORK_NAME}");
                    create_network(
                        &qemu_conn,
                        BOOT_NETWORK_NAME,
                        BOOT_NETWORK_BRIDGE,
                        BOOT_NETWORK_IP,
                        BOOT_NETWORK_NETMASK,
                        BOOT_NETWORK_DHCP_START,
                        BOOT_NETWORK_DHCP_END,
                        BOOT_NETWORK_HTTP_SERVER,
                    )?;
                }

                // Create the isolated network
                if qemu_conn
                    .list_networks()?
                    .iter()
                    .any(|net| net == ISOLATED_NETWORK_NAME)
                {
                    println!("Network already exists: {ISOLATED_NETWORK_NAME}");
                } else {
                    println!("Creating network: {ISOLATED_NETWORK_NAME}");
                    create_isolated_network(
                        &qemu_conn,
                        ISOLATED_NETWORK_NAME,
                        ISOLATED_NETWORK_BRIDGE,
                    )?;
                }
            }

            Commands::Up { config_file } => {
                term_msg_surround("Building environment");

                // TODO: allow config file to be specified.
                let _config_file = config_file;

                let qemu_conn = Arc::new(qemu.connect()?);

                let config = Config::load(&sherpa.config_path)?;
                let manifest = Manifest::load_file()?;

                generate_ssh_keypair()?;

                // Create a mapping of device name to device id.
                let dev_id_map: HashMap<String, u8> = manifest
                    .devices
                    .iter()
                    .map(|d| (d.name.clone(), d.id))
                    .collect();

                // let mut ztp_disks: Vec<ZtpDisk> = vec![];
                let mut copy_disks: Vec<CloneDisk> = vec![];
                let mut domains: Vec<DomainTemplate> = vec![];
                for device in &manifest.devices {
                    let vm_name = format!("{}-{}", device.name, manifest.id);
                    let mac_oui = match device.device_model {
                        DeviceModels::AristaVeos => ARISTA_OUI,
                        DeviceModels::CiscoAsav
                        | DeviceModels::CiscoCat8000v
                        | DeviceModels::CiscoCat9000v
                        | DeviceModels::CiscoCsr1000v
                        | DeviceModels::CiscoIosv
                        | DeviceModels::CiscoIosvl2
                        | DeviceModels::CiscoIosxrv9000
                        | DeviceModels::CiscoNexus9300v => CISCO_OUI,
                        DeviceModels::JuniperVjunosRouter | DeviceModels::JuniperVjunosSwitch => {
                            JUNIPER_OUI
                        }
                        _ => KVM_OUI,
                    };
                    let device_model = config
                        .device_models
                        .iter()
                        .find(|d| d.name == device.device_model)
                        .ok_or_else(|| {
                            anyhow::anyhow!("Device model not found: {}", device.device_model)
                        })?;

                    let mut interfaces: Vec<Interface> = vec![];

                    if device_model.management_interface {
                        interfaces.push(Interface {
                            name: "mgmt".to_owned(),
                            num: 0,
                            mtu: device_model.interface_mtu,
                            mac_address: random_mac(mac_oui.to_string()),
                            connection_type: ConnectionTypes::Management,
                            connection_map: None,
                        });
                    }
                    if device_model.reserved_interface_count > 0 {
                        for i in 0..device_model.reserved_interface_count {
                            interfaces.push(Interface {
                                name: "reserved".to_owned(),
                                num: i,
                                mtu: device_model.interface_mtu,
                                mac_address: random_mac(KVM_OUI.to_string()),
                                connection_type: ConnectionTypes::Reserved,
                                connection_map: None,
                            });
                        }
                    }
                    for i in 0..device_model.interface_count {
                        match &manifest.connections {
                            Some(connections) => {
                                for c in connections {
                                    // Device is source in manifest
                                    if c.device_a == device.name && i == c.interface_a {
                                        let source_id =
                                            dev_id_map.get(&c.device_b).ok_or_else(|| {
                                                anyhow::anyhow!(
                                                    "Connection device_b not found: {}",
                                                    c.device_b
                                                )
                                            })?;
                                        let connection_map = ConnectionMap {
                                            local_id: device.id,
                                            local_port: id_to_port(i),
                                            local_loopback: get_ip(device.id).to_string(),
                                            source_id: source_id.to_owned(),
                                            source_port: id_to_port(c.interface_b),
                                            source_loopback: get_ip(source_id.to_owned())
                                                .to_string(),
                                        };
                                        interfaces.push(Interface {
                                            name: format!("{}{}", device_model.interface_prefix, i),
                                            num: i,
                                            mtu: device_model.interface_mtu,
                                            mac_address: random_mac(KVM_OUI.to_string()),
                                            connection_type: ConnectionTypes::Peer,
                                            connection_map: Some(connection_map),
                                        })
                                    // Device is destination in manifest
                                    } else if c.device_b == device.name && i == c.interface_b {
                                        let source_id =
                                            dev_id_map.get(&c.device_a).ok_or_else(|| {
                                                anyhow::anyhow!(
                                                    "Connection device_a not found: {}",
                                                    c.device_a
                                                )
                                            })?;
                                        let connection_map = ConnectionMap {
                                            local_id: device.id,
                                            local_port: id_to_port(i),
                                            local_loopback: get_ip(device.id).to_string(),
                                            source_id: source_id.to_owned(),
                                            source_port: id_to_port(c.interface_a),
                                            source_loopback: get_ip(source_id.to_owned())
                                                .to_string(),
                                        };
                                        interfaces.push(Interface {
                                            name: format!("{}{}", device_model.interface_prefix, i),
                                            num: i,
                                            mtu: device_model.interface_mtu,
                                            mac_address: random_mac(KVM_OUI.to_string()),
                                            connection_type: ConnectionTypes::Peer,
                                            connection_map: Some(connection_map),
                                        })
                                    } else {
                                        // Interface not defined in manifest so disable.
                                        interfaces.push(Interface {
                                            name: format!("{}{}", device_model.interface_prefix, i),
                                            num: i,
                                            mtu: device_model.interface_mtu,
                                            mac_address: random_mac(KVM_OUI.to_string()),
                                            connection_type: ConnectionTypes::Disabled,
                                            connection_map: None,
                                        })
                                    }
                                }
                            }
                            None => interfaces.push(Interface {
                                name: format!("{}{}", device_model.interface_prefix, i),
                                num: i,
                                mtu: device_model.interface_mtu,
                                mac_address: random_mac(KVM_OUI.to_string()),
                                connection_type: ConnectionTypes::Disabled,
                                connection_map: None,
                            }),
                        }
                    }

                    let src_boot_disk = format!(
                        "{}/{}/{}/virtioa.qcow2",
                        sherpa.boxes_dir, device_model.name, device_model.version
                    );
                    let dst_boot_disk = format!("{STORAGE_POOL_PATH}/{vm_name}.qcow2");
                    copy_disks.push(CloneDisk {
                        src: src_boot_disk,
                        dst: dst_boot_disk.clone(),
                    });

                    // CDROM ISO
                    let (mut src_cdrom_iso, mut dst_cdrom_iso) = match &device_model.cdrom {
                        Some(src_iso) => {
                            let src = format!(
                                "{}/{}/{}/{}",
                                sherpa.boxes_dir, device_model.name, device_model.version, src_iso
                            );
                            let dst = format!("{STORAGE_POOL_PATH}/{vm_name}.iso");
                            (Some(src), Some(dst))
                        }
                        None => (None, None),
                    };

                    let user = User::default()?;
                    if device_model.ztp_enable && device_model.ztp_method == ZtpMethods::Cdrom {
                        // generate the template
                        println!("Creating ZTP config {}", device.name);
                        let mut user = user.clone();
                        let dir = format!("{}/{}", TEMP_DIR, vm_name);

                        match device_model.os_variant {
                            OsVariants::Iosxe => {
                                let key_hash = pub_ssh_key_to_md5_hash(&user.ssh_public_key.key)?;
                                user.ssh_public_key.key = key_hash;
                                let t = CiscoIosXeZtpTemplate {
                                    hostname: device.name.clone(),
                                    users: vec![user],
                                    mgmt_interface: "GigabitEthernet1".to_owned(),
                                };
                                let rendered_template = t.render()?;
                                let ztp_config = format!("{dir}/{CISCO_IOSXE_ZTP_CONFIG}");
                                create_dir(&dir)?;
                                create_file(&ztp_config, rendered_template)?;
                                create_ztp_iso(&format!("{}/{}", dir, ZTP_ISO), dir)?
                            }
                            OsVariants::Linux => {
                                let t = CloudInitTemplate {
                                    hostname: device.name.clone(),
                                    users: vec![user],
                                };
                                let rendered_template = t.render()?;
                                let user_data = format!("{dir}/{CLOUD_INIT_USER_DATA}");
                                let meta_data = format!("{dir}/{CLOUD_INIT_META_DATA}");
                                create_dir(&dir)?;
                                create_file(&user_data, rendered_template)?;
                                create_file(&meta_data, "".to_string())?;
                                create_ztp_iso(&format!("{}/{}", dir, ZTP_ISO), dir)?
                            }
                            _ => {
                                println!(
                                    "CDROM ZTP method not supported for {}",
                                    device_model.name
                                );
                            }
                        };
                        src_cdrom_iso = Some(format!("{TEMP_DIR}/{vm_name}/{ZTP_ISO}"));
                        dst_cdrom_iso = Some(format!("{STORAGE_POOL_PATH}/{vm_name}.iso"));
                    }

                    // Other ISO
                    if dst_cdrom_iso.is_some() {
                        copy_disks.push(CloneDisk {
                            // These should always have a value.
                            src: src_cdrom_iso.unwrap(),
                            dst: dst_cdrom_iso.clone().unwrap(),
                        })
                    }
                    let domain = DomainTemplate {
                        qemu_bin: config.qemu_bin.clone(),
                        name: vm_name,
                        memory: device_model.memory,
                        cpu_architecture: device_model.cpu_architecture.clone(),
                        machine_type: device_model.machine_type.clone(),
                        cpu_count: device_model.cpu_count,
                        vmx_enabled: device_model.vmx_enabled,
                        bios: device_model.bios.clone(),
                        boot_disk: dst_boot_disk,
                        cdrom: dst_cdrom_iso,
                        interfaces,
                        interface_type: device_model.interface_type.clone(),
                        loopback_ipv4: get_ip(device.id).to_string(),
                        telnet_port: TELNET_PORT,
                    };

                    domains.push(domain);
                }

                // Clone disks in parallel
                term_msg_underline("Cloning Disks");
                let disk_handles: Vec<_> = copy_disks
                    .into_iter()
                    .map(|disk| {
                        let qemu_conn = Arc::clone(&qemu_conn);
                        thread::spawn(move || -> Result<()> {
                            println!("Cloning disk \n  from: {} \n    to: {}", disk.src, disk.dst);
                            clone_disk(&qemu_conn, &disk.src, &disk.dst).with_context(|| {
                                format!("Failed to clone disk from: {} to: {}", disk.src, disk.dst)
                            })?;
                            println!("Cloned disk \n  from: {} \n    to: {}", disk.src, disk.dst);
                            Ok(())
                        })
                    })
                    .collect();

                // Wait for all threads to complete
                for handle in disk_handles {
                    handle
                        .join()
                        .map_err(|e| anyhow::anyhow!("Error cloning disk: {:?}", e))??;
                }

                // Build domains in parallel
                term_msg_underline("Creating VMs");
                let vm_handles: Vec<_> = domains
                    .into_iter()
                    .map(|domain| {
                        let qemu_conn = Arc::clone(&qemu_conn);
                        thread::spawn(move || -> Result<()> {
                            let rendered_xml = domain.render().with_context(|| {
                                format!("Failed to render XML for VM: {}", domain.name)
                            })?;
                            println!("Creating VM: {}", domain.name);
                            create_vm(&qemu_conn, &rendered_xml)
                                .with_context(|| format!("Failed to create VM: {}", domain.name))?;
                            println!("Created VM: {}", domain.name);
                            Ok(())
                        })
                    })
                    .collect();

                // Wait for all threads to complete
                for handle in vm_handles {
                    handle
                        .join()
                        .map_err(|e| anyhow::anyhow!("Error creating VM: {:?}", e))??;
                }

                // Check if VMs are ready
                term_msg_underline("Checking VM Readiness");
                let start_time = Instant::now();
                let timeout = Duration::from_secs(READINESS_TIMEOUT); // 10 minutes
                let mut connected_devices = std::collections::HashSet::new();

                while start_time.elapsed() < timeout
                    && connected_devices.len() < manifest.devices.len()
                {
                    for device in &manifest.devices {
                        if connected_devices.contains(&device.name) {
                            continue;
                        }

                        let vm_name = format!("{}-{}", device.name, manifest.id);
                        if let Some(vm_ip) = get_mgmt_ip(&qemu_conn, &vm_name)? {
                            match tcp_connect(&vm_ip, SSH_PORT)? {
                                true => {
                                    println!("{} is ready", device.name);
                                    connected_devices.insert(device.name.clone());
                                }
                                false => {
                                    println!("Waiting for {}", device.name);
                                }
                            }
                        } else {
                            println!("Waiting for {}", device.name);
                        }
                    }

                    if connected_devices.len() < manifest.devices.len() {
                        sleep(Duration::from_secs(READINESS_SLEEP));
                    }
                }

                if connected_devices.len() == manifest.devices.len() {
                    println!("All devices are ready!");
                } else {
                    println!("Timeout reached. Not all devices are ready.");
                    for device in &manifest.devices {
                        if !connected_devices.contains(&device.name) {
                            println!("Device is not ready: {}", device.name);
                        }
                    }
                }
            }
            Commands::Down => {
                term_msg_surround("Suspending environment");

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;

                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&manifest.id) {
                        if domain.is_active()? {
                            domain.suspend()?;
                            println!("Suspended: {vm_name}");
                        } else {
                            println!("Virtual machine not running: {vm_name}");
                        }
                    }
                }
            }
            Commands::Resume => {
                term_msg_surround("Resuming environment");

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;

                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&manifest.id) {
                        match domain.get_state() {
                            Ok((state, _reason)) => {
                                if state == virt::sys::VIR_DOMAIN_PAUSED {
                                    domain.resume()?;
                                    println!("Resumed: {vm_name}");
                                } else if state == virt::sys::VIR_DOMAIN_RUNNING {
                                    println!("Virtual machine already running: {vm_name}");
                                } else {
                                    println!(
                                        "Virtual machine not paused (state: {}): {}",
                                        state, vm_name
                                    );
                                }
                            }
                            Err(e) => anyhow::bail!("Failed to get state for {vm_name}: {e}"),
                        }
                    }
                }
            }
            Commands::Destroy => {
                term_msg_surround("Destroying environment");

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;

                for domain in domains {
                    let vm_name = domain.get_name()?;
                    if vm_name.contains(&manifest.id) && domain.is_active()? {
                        // EUFI domains will have an NVRAM file that must be deleted.
                        let nvram_flag = sys::VIR_DOMAIN_UNDEFINE_NVRAM;
                        domain.undefine_flags(nvram_flag)?;
                        domain.destroy()?;
                        println!("Destroyed VM: {vm_name}");

                        // HDD
                        let hdd_name = format!("{vm_name}.qcow2");
                        delete_disk(&qemu_conn, &hdd_name)?;
                        println!("Deleted HDD: {hdd_name}");

                        // ISO
                        let iso_name = format!("{vm_name}.iso");
                        if file_exists(&format!("{STORAGE_POOL_PATH}/{iso_name}")) {
                            delete_disk(&qemu_conn, &iso_name)?;
                            println!("Deleted ISO: {iso_name}");
                        }
                    }
                }
                if dir_exists(TEMP_DIR) {
                    fs::remove_dir_all(TEMP_DIR)?;
                    println!("Deleted directory: {TEMP_DIR}");
                }
            }
            Commands::Inspect => {
                let manifest = Manifest::load_file()?;

                term_msg_surround(&format!("Sherpa Environment - {}", manifest.id));

                let qemu_conn = qemu.connect()?;

                let domains = qemu_conn.list_all_domains(0)?;
                let pool = StoragePool::lookup_by_name(&qemu_conn, STORAGE_POOL)?;
                for device in manifest.devices {
                    let device_name = format!("{}-{}", device.name, manifest.id);
                    if let Some(domain) = domains
                        .iter()
                        .find(|d| d.get_name().unwrap_or_default() == device_name)
                    {
                        term_msg_underline(&device.name);
                        println!("Domain: {}", device_name);
                        println!("Model: {}", device.device_model);
                        println!("Active: {:#?}", domain.is_active()?);
                        if let Some(vm_ip) = get_mgmt_ip(&qemu_conn, &device_name)? {
                            println!("Mgmt IP: {vm_ip}");
                        }
                        for volume in pool.list_volumes()? {
                            if volume.contains(&device_name) {
                                println!("Disk: {volume}");
                            }
                        }
                    }
                }
            }
            Commands::Import {
                src,
                version,
                model,
                latest,
            } => {
                term_msg_surround("Importing disk image");

                if !file_exists(src) {
                    anyhow::bail!("File does not exist: {}", src);
                }

                let dst_path = format!("{}/{}", sherpa.boxes_dir, model);
                let dst_version_dir = format!("{dst_path}/{version}");
                let dst_latest_dir = format!("{dst_path}/latest");

                create_dir(&dst_version_dir)?;
                create_dir(&dst_latest_dir)?;

                let dst_version_disk = format!("{dst_version_dir}/virtioa.qcow2");

                if !file_exists(&dst_version_disk) {
                    println!("Copying file from: {} to: {}", src, dst_version_disk);
                    copy_file(src, &dst_version_disk)?;
                    println!("Copied file from: {} to: {}", src, dst_version_disk);
                } else {
                    println!("File already exists: {}", dst_version_disk);
                }

                if *latest {
                    let dst_latest_disk = format!("{dst_latest_dir}/virtioa.qcow2");
                    println!("Copying file from: {} to: {}", src, dst_latest_disk);
                    copy_file(src, &dst_latest_disk)?;
                    println!("Copied file from: {} to: {}", src, dst_latest_disk);
                }

                println!("Setting base box files to read-only");
                fix_permissions_recursive(&format!("{CONFIG_DIR}/{BOXES_DIR}"))?;
            }
            Commands::Doctor { boxes } => {
                if *boxes {
                    term_msg_surround("Fixing base box permissions");

                    fix_permissions_recursive(&format!("{CONFIG_DIR}/{BOXES_DIR}"))?;
                }
            }
            Commands::Cleaner {
                all,
                disks,
                networks,
            } => {
                if *all {
                    // term_msg_surround("Cleaning environment");
                    term_msg_surround("Not implemented");
                } else if *disks {
                    term_msg_surround("Cleaning disks");
                    let manifest = Manifest::load_file()?;

                    let qemu_conn = qemu.connect()?;

                    let pool = StoragePool::lookup_by_name(&qemu_conn, STORAGE_POOL)?;
                    for volume in pool.list_volumes()? {
                        if volume.contains(&manifest.id) {
                            println!("Deleting disk: {}", volume);
                            let vol = StorageVol::lookup_by_name(&pool, &volume)?;
                            vol.delete(0)?;
                            println!("Deleted disk: {}", volume);
                        }
                    }
                } else if *networks {
                    // term_msg_surround("Cleaning networks");
                    term_msg_surround("Not implemented");
                }
            }
            Commands::Console { name } => {
                term_msg_surround(&format!("Connecting to: {name}"));

                let manifest = Manifest::load_file()?;

                // Find the device in the manifest
                let device = manifest
                    .devices
                    .iter()
                    .find(|d| d.name == *name)
                    .ok_or_else(|| anyhow::anyhow!("Device not found: {}", name))?;

                let status = Command::new("telnet")
                    .arg(get_ip(device.id).to_string())
                    .arg(TELNET_PORT.to_string())
                    .status()?;

                if !status.success() {
                    eprintln!("Telnet connection failed");
                    if let Some(code) = status.code() {
                        eprintln!("Exit code: {}", code);
                    }
                }
            }
            Commands::Ssh { name } => {
                term_msg_surround(&format!("Connecting to: {name}"));

                let manifest = Manifest::load_file()?;

                let qemu_conn = qemu.connect()?;

                if let Some(vm_ip) = get_mgmt_ip(&qemu_conn, &format!("{}-{}", name, manifest.id))?
                {
                    let status = Command::new("ssh")
                        .arg(&format!("{SHERPA_USERNAME}@{vm_ip}"))
                        .arg("-i")
                        .arg(&format!("{TEMP_DIR}/{SHERPA_SSH_PRIVATE_KEY_FILE}"))
                        .arg("-o")
                        .arg("StrictHostKeyChecking=no")
                        .arg("-o")
                        .arg("UserKnownHostsFile=/dev/null")
                        .status()?;

                    if !status.success() {
                        eprintln!("SSH connection failed");
                        if let Some(code) = status.code() {
                            eprintln!("Exit code: {}", code);
                        }
                    }
                } else {
                    eprintln!("No IP address found for {name}")
                }
            }
        }
        Ok(())
    }
}
