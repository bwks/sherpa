use anyhow::Result;
use askama::Template;

use data::{
    BiosTypes, CloneDisk, Config, ConnectionTypes, CpuArchitecture, CpuModels, DeviceDisk,
    DeviceModels, DiskBuses, DiskDevices, DiskDrivers, DiskFormats, DiskTargets, Dns, Interface,
    InterfaceTypes, MachineTypes, MgmtInterfaces, QemuCommand, Sherpa, User, ZtpTemplates,
};
use konst::{
    ARISTA_VEOS_ZTP_SCRIPT, ARISTA_ZTP_DIR, ARUBA_ZTP_CONFIG, ARUBA_ZTP_DIR, BOOT_SERVER_MAC,
    BOOT_SERVER_NAME, CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_ZTP_CONFIG, CISCO_ZTP_DIR,
    CONTAINER_DISK_NAME, CUMULUS_ZTP_CONFIG, CUMULUS_ZTP_DIR, JUNIPER_ZTP_DIR, JUNIPER_ZTP_SCRIPT,
    MTU_JUMBO_INT, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_EXT4_500M, SHERPA_PASSWORD_HASH,
    SHERPA_STORAGE_POOL_PATH, SHERPA_USERNAME, TELNET_PORT, TEMP_DIR, ZTP_DIR, ZTP_JSON,
};
use template::{
    ArubaAoscxTemplate, BootServer, CiscoIosXeZtpTemplate, CiscoIosvZtpTemplate,
    Contents as IgnitionFileContents, CumulusLinuxZtpTemplate, DomainTemplate,
    File as IgnitionFile, FileSystem as IgnitionFileSystem, IgnitionConfig, Link as IgnitionLink,
    Unit as IgnitionUnit, User as IgnitionUser, arista_veos_ztp_script,
    juniper_vevolved_ztp_script,
};
use util::{
    base64_encode, copy_file, copy_to_ext4_image, create_dir, create_file, get_ip,
    pub_ssh_key_to_md5_hash, term_msg_underline,
};

pub fn create_ztp_files(sherpa_user: &User, dns: &Dns) -> Result<ZtpTemplates> {
    // Create ZTP files
    term_msg_underline("Creating ZTP configs");

    // Aristra vEOS
    let arista_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{ARISTA_ZTP_DIR}");
    create_dir(&arista_dir)?;

    let arista_ztp_file = format!("{arista_dir}/{ARISTA_VEOS_ZTP_SCRIPT}");
    let arista_ztp_script = arista_veos_ztp_script();
    create_file(&arista_ztp_file, arista_ztp_script.clone())?;

    // Aruba AOS
    let aruba_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{ARUBA_ZTP_DIR}");
    create_dir(&aruba_dir)?;

    // TODO: Aruba USB ZTP config
    // let aruba_ztp_file = format!("{aruba_dir}/{ARUBA_ZTP_SCRIPT}");
    // let aruba_ztp_script = aruba_aoscx_ztp_script();
    // create_file(&aruba_ztp_file, aruba_ztp_script.clone())?;

    let aruba_template = ArubaAoscxTemplate {
        hostname: "aos-ztp".to_owned(),
        user: sherpa_user.clone(),
        dns: dns.clone(),
    };
    let aruba_rendered_template = aruba_template.render()?;
    let aruba_ztp_config = format!("{aruba_dir}/{ARUBA_ZTP_CONFIG}");
    create_file(&aruba_ztp_config, aruba_rendered_template.clone())?;

    // Cumulus Linux
    let cumulus_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{CUMULUS_ZTP_DIR}");
    create_dir(&cumulus_dir)?;

    let cumulus_template = CumulusLinuxZtpTemplate {
        hostname: "cumulus-ztp".to_owned(),
        user: sherpa_user.clone(),
        dns: dns.clone(),
    };
    let cumulus_rendered_template = cumulus_template.render()?;
    let cumulus_ztp_config = format!("{cumulus_dir}/{CUMULUS_ZTP_CONFIG}");
    create_file(&cumulus_ztp_config, cumulus_rendered_template.clone())?;

    // Cisco
    let cisco_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{CISCO_ZTP_DIR}");
    create_dir(&cisco_dir)?;
    let mut cisco_user = sherpa_user.clone();
    cisco_user.ssh_public_key.key = pub_ssh_key_to_md5_hash(&cisco_user.ssh_public_key.key)?;

    // IOSXE
    let cisco_iosxe_template = CiscoIosXeZtpTemplate {
        hostname: "iosxe-ztp".to_owned(),
        user: cisco_user.clone(),
        mgmt_interface: MgmtInterfaces::GigabitEthernet1.to_string(),
        dns: dns.clone(),
    };
    let iosxe_rendered_template = cisco_iosxe_template.render()?;
    let cisco_iosxe_ztp_config = format!("{cisco_dir}/{CISCO_IOSXE_ZTP_CONFIG}");
    create_file(&cisco_iosxe_ztp_config, iosxe_rendered_template.clone())?;

    // IOSv
    let cisco_iosv_template = CiscoIosvZtpTemplate {
        hostname: "iosv-ztp".to_owned(),
        user: cisco_user.clone(),
        mgmt_interface: MgmtInterfaces::GigabitEthernet0_0.to_string(),
        dns: dns.clone(),
    };
    let iosv_rendered_template = cisco_iosv_template.render()?;
    let cisco_iosv_ztp_config = format!("{cisco_dir}/{CISCO_IOSV_ZTP_CONFIG}");
    create_file(&cisco_iosv_ztp_config, iosv_rendered_template.clone())?;

    // Juniper vevolved
    let juniper_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{JUNIPER_ZTP_DIR}");
    create_dir(&juniper_dir)?;

    let juniper_vjunos_script = juniper_vevolved_ztp_script();
    let juniper_vjunos_ztp_config = format!("{juniper_dir}/{JUNIPER_ZTP_SCRIPT}");
    create_file(&juniper_vjunos_ztp_config, juniper_vjunos_script.clone())?;

    Ok(ZtpTemplates {
        arista_eos: arista_ztp_script,
        // aruba_aos: aruba_ztp_script,
        aruba_aos: aruba_rendered_template,
        cumulus_linux: cumulus_rendered_template,
        cisco_iosv: iosv_rendered_template,
        cisco_iosxe: iosxe_rendered_template,
        juniper_vjunos: juniper_vjunos_script,
    })
}

pub fn create_boot_server(
    sherpa: &Sherpa,
    config: &Config,
    lab_name: &str,
    lab_id: &str,
    user: &User,
    ztp_templates: &ZtpTemplates,
) -> Result<BootServer> {
    let boot_server_name = format!("{BOOT_SERVER_NAME}-{lab_name}-{lab_id}");
    let dir = format!("{TEMP_DIR}/{boot_server_name}");
    let ignition_user = IgnitionUser {
        name: user.username.clone(),
        password_hash: SHERPA_PASSWORD_HASH.to_owned(),
        ssh_authorized_keys: vec![format!(
            "{} {}",
            user.ssh_public_key.algorithm, user.ssh_public_key.key
        )],
        groups: vec!["wheel".to_owned(), "docker".to_owned()],
    };
    let hostname_file = IgnitionFile {
        path: "/etc/hostname".to_owned(),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:,{BOOT_SERVER_NAME}")),
        ..Default::default()
    };
    let container_disk = IgnitionFileSystem::default();

    let ztp_interface = IgnitionFile::ztp_interface(&config)?;
    let unit_webdir = IgnitionUnit::webdir();
    let unit_dnsmasq = IgnitionUnit::dnsmasq();
    let _srlinux_unit = IgnitionUnit::srlinux();
    let container_disk_mount = IgnitionUnit::mount_container_disk();
    // files
    let sudo_config_base64 = base64_encode(&format!("{SHERPA_USERNAME} ALL=(ALL) NOPASSWD: ALL"));
    let sudo_config_file = IgnitionFile {
        path: format!("/etc/sudoers.d/{SHERPA_USERNAME}"),
        mode: 440,
        contents: IgnitionFileContents::new(&format!("data:;base64,{sudo_config_base64}")),
        ..Default::default()
    };

    let arista_ztp_base64 = base64_encode(&ztp_templates.arista_eos);
    let arista_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{ARISTA_ZTP_DIR}/{ARISTA_VEOS_ZTP_SCRIPT}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{arista_ztp_base64}")),
        ..Default::default()
    };

    let aruba_ztp_base64 = base64_encode(&ztp_templates.aruba_aos);
    let aruba_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{ARUBA_ZTP_DIR}/{ARUBA_ZTP_CONFIG}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{aruba_ztp_base64}")),
        ..Default::default()
    };

    let cumulus_ztp_base64 = base64_encode(&ztp_templates.cumulus_linux);
    let cumulus_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{CUMULUS_ZTP_DIR}/{CUMULUS_ZTP_CONFIG}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{cumulus_ztp_base64}")),
        ..Default::default()
    };
    let iosxe_ztp_base64 = base64_encode(&ztp_templates.cisco_iosxe);
    let iosxe_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{CISCO_ZTP_DIR}/{CISCO_IOSXE_ZTP_CONFIG}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{iosxe_ztp_base64}")),
        ..Default::default()
    };
    let iosv_ztp_base64 = base64_encode(&ztp_templates.cisco_iosv);
    let iosv_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{CISCO_ZTP_DIR}/{CISCO_IOSV_ZTP_CONFIG}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{iosv_ztp_base64}")),
        ..Default::default()
    };
    let juniper_vjunos_ztp_base64 = base64_encode(&ztp_templates.juniper_vjunos);
    let juniper_vjunos_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{JUNIPER_ZTP_DIR}/{JUNIPER_ZTP_SCRIPT}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{juniper_vjunos_ztp_base64}")),
        ..Default::default()
    };
    let ignition_config = IgnitionConfig::new(
        vec![ignition_user],
        vec![
            sudo_config_file,
            hostname_file,
            // ztp_interface,
            IgnitionFile::disable_resolved(),
            IgnitionFile::disable_updates(),
            IgnitionFile::docker_compose_raw(),
            IgnitionFile::docker_compose_conf(),
            IgnitionFile::dnsmasq_config(),
            IgnitionFile::systemd_noop(),
            arista_ztp_file,
            aruba_ztp_file,
            cumulus_ztp_file,
            iosxe_ztp_file,
            iosv_ztp_file,
            juniper_vjunos_ztp_file,
        ],
        vec![IgnitionLink::docker_compose_raw()],
        vec![
            IgnitionUnit::systemd_resolved(),
            IgnitionUnit::systemd_update_timer(),
            IgnitionUnit::systemd_update_service(),
            unit_webdir,
            unit_dnsmasq,
            container_disk_mount,
            // srlinux_unit
        ],
        vec![container_disk],
    );
    let flatcar_config = ignition_config.to_json_pretty()?;
    let src_ztp_file = format!("{dir}/{ZTP_JSON}");
    let dst_ztp_file = format!("{SHERPA_STORAGE_POOL_PATH}/{boot_server_name}-cfg.ign");

    create_dir(&dir)?;
    create_file(&src_ztp_file, flatcar_config)?;

    let mut copy_disks = vec![];
    copy_disks.push(CloneDisk {
        src: src_ztp_file,
        dst: dst_ztp_file.clone(),
    });

    let src_boot_disk = format!(
        "{}/{}/{}/virtioa.qcow2",
        sherpa.boxes_dir,
        DeviceModels::FlatcarLinux,
        "latest"
    );
    let dst_boot_disk = format!("{SHERPA_STORAGE_POOL_PATH}/{boot_server_name}-hdd.qcow2");

    copy_disks.push(CloneDisk {
        src: src_boot_disk,
        dst: dst_boot_disk.clone(),
    });
    // Copy a blank disk to to .tmp directory
    let src_data_disk = format!(
        "{}/{}/{}",
        &sherpa.boxes_dir, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_EXT4_500M
    );
    let dst_disk = format!("{dir}/{boot_server_name}-{CONTAINER_DISK_NAME}");

    copy_file(&src_data_disk, &dst_disk)?;

    // let disk_files: Vec<&str> = manifest_binary_disk_files
    //     .iter()
    //     .map(|x| x.source.as_str())
    //     .collect();

    let disk_files = vec![
        "/home/bradmin/.sherpa/containers/kea-dhcp4.tar.gz",
        "/home/bradmin/.sherpa/containers/dnsmasq.tar.gz",
        "/home/bradmin/.sherpa/containers/webdir.tar.gz",
    ];
    // Copy to container image into the container disk
    copy_to_ext4_image(disk_files, &dst_disk, "/")?;

    let src_config_disk = Some(dst_disk.to_owned());
    let dst_config_disk = Some(format!(
        "{SHERPA_STORAGE_POOL_PATH}/{boot_server_name}-{CONTAINER_DISK_NAME}"
    ));

    copy_disks.push(CloneDisk {
        src: src_config_disk.unwrap(),
        dst: dst_config_disk.clone().unwrap(),
    });

    let device_disks: Vec<DeviceDisk> = vec![
        DeviceDisk {
            disk_device: DiskDevices::File,
            driver_name: DiskDrivers::Qemu,
            driver_format: DiskFormats::Qcow2,
            src_file: dst_boot_disk.clone(),
            target_dev: DiskTargets::target(&DiskBuses::Sata, 0)?,
            target_bus: DiskBuses::Sata,
        },
        DeviceDisk {
            disk_device: DiskDevices::File,
            driver_name: DiskDrivers::Qemu,
            driver_format: DiskFormats::Raw,
            src_file: dst_config_disk.unwrap(),
            target_dev: DiskTargets::target(&DiskBuses::Sata, 1)?,
            target_bus: DiskBuses::Sata,
        },
    ];

    let domain = DomainTemplate {
        qemu_bin: config.qemu_bin.clone(),
        name: boot_server_name.to_owned(),
        memory: 2048,
        cpu_architecture: CpuArchitecture::default(),
        cpu_model: CpuModels::default(),
        machine_type: MachineTypes::default(),
        cpu_count: 2,
        vmx_enabled: false,
        bios: BiosTypes::default(),
        disks: device_disks,
        interfaces: vec![Interface {
            name: "mgmt".to_owned(),
            num: 0,
            mtu: MTU_JUMBO_INT,
            mac_address: BOOT_SERVER_MAC.to_owned(),
            connection_type: ConnectionTypes::Management,
            interface_connection: None,
        }],
        interface_type: InterfaceTypes::Virtio,
        loopback_ipv4: get_ip(255).to_string(),
        telnet_port: TELNET_PORT,
        qemu_commands: QemuCommand::ignition_config(&dst_ztp_file),
    };

    Ok(BootServer {
        template: domain,
        copy_disks,
    })
}
