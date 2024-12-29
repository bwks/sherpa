use anyhow::Result;
use rinja::Template;

use crate::core::konst::{
    ARISTA_VEOS_ZTP_SCRIPT, ARISTA_ZTP_DIR, ARUBA_ZTP_CONFIG, ARUBA_ZTP_DIR, BOOT_SERVER_MAC,
    BOOT_SERVER_NAME, CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_ZTP_CONFIG, CISCO_ZTP_DIR,
    CUMULUS_ZTP_CONFIG, CUMULUS_ZTP_DIR, JUNIPER_ZTP_CONFIG, JUNIPER_ZTP_DIR, MTU_JUMBO_INT,
    SHERPA_STORAGE_POOL_PATH, SHERPA_USERNAME, TELNET_PORT, TEMP_DIR, ZTP_DIR, ZTP_JSON,
};
use crate::core::{Config, Sherpa};
use crate::data::{
    BiosTypes, BootServer, CloneDisk, ConnectionTypes, CpuArchitecture, CpuModels, DeviceDisk,
    DeviceModels, DiskBuses, DiskDevices, DiskDrivers, DiskFormats, DiskTargets, Dns, Interface,
    InterfaceTypes, MachineTypes, MgmtInterfaces, QemuCommand, User, ZtpTemplates,
};
use crate::libvirt::DomainTemplate;
use crate::template::{
    arista_veos_ztp_script, ArubaAoscxTemplate, CiscoIosXeZtpTemplate, CiscoIosvZtpTemplate,
    Contents as IgnitionFileContents, CumulusLinuxZtpTemplate, File as IgnitionFile,
    IgnitionConfig, JunipervJunosZtpTemplate, Unit as IgnitionUnit, User as IgnitionUser,
};
use crate::util::{
    base64_encode, create_dir, create_file, get_ip, pub_ssh_key_to_md5_hash, term_msg_underline,
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
        users: vec![sherpa_user.clone()],
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

    // Juniper vrouter
    let juniper_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{JUNIPER_ZTP_DIR}");
    create_dir(&juniper_dir)?;

    let juniper_vjunos_template = JunipervJunosZtpTemplate {
        hostname: "vjunos-ztp".to_owned(),
        user: sherpa_user.clone(),
        mgmt_interface: MgmtInterfaces::Re0Mgmt0.to_string(),
    };
    let juniper_vjunos_rendered_template = juniper_vjunos_template.render()?;
    let juniper_vjunos_ztp_config = format!("{juniper_dir}/{JUNIPER_ZTP_CONFIG}");
    create_file(
        &juniper_vjunos_ztp_config,
        juniper_vjunos_rendered_template.clone(),
    )?;

    Ok(ZtpTemplates {
        arista_eos: arista_ztp_script,
        // aruba_aos: aruba_ztp_script,
        aruba_aos: aruba_rendered_template,
        cumulus_linux: cumulus_rendered_template,
        cisco_iosv: iosv_rendered_template,
        cisco_iosxe: iosxe_rendered_template,
        juniper_vjunos: juniper_vjunos_rendered_template,
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
    };
    let unit_webdir = IgnitionUnit::webdir();
    let unit_tftp = IgnitionUnit::tftpd();
    let _srlinux_unit = IgnitionUnit::srlinux();

    // files
    let sudo_config_base64 = base64_encode(&format!("{SHERPA_USERNAME} ALL=(ALL) NOPASSWD: ALL"));
    let sudo_config_file = IgnitionFile {
        path: format!("/etc/sudoers.d/{SHERPA_USERNAME}"),
        mode: 440,
        contents: IgnitionFileContents::new(&format!("data:;base64,{sudo_config_base64}")),
    };

    let arista_ztp_base64 = base64_encode(&ztp_templates.arista_eos);
    let arista_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{ARISTA_ZTP_DIR}/{ARISTA_VEOS_ZTP_SCRIPT}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{arista_ztp_base64}")),
    };

    let aruba_ztp_base64 = base64_encode(&ztp_templates.aruba_aos);
    let aruba_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{ARUBA_ZTP_DIR}/{ARUBA_ZTP_CONFIG}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{aruba_ztp_base64}")),
    };

    let cumulus_ztp_base64 = base64_encode(&ztp_templates.cumulus_linux);
    let cumulus_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{CUMULUS_ZTP_DIR}/{CUMULUS_ZTP_CONFIG}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{cumulus_ztp_base64}")),
    };
    let iosxe_ztp_base64 = base64_encode(&ztp_templates.cisco_iosxe);
    let iosxe_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{CISCO_ZTP_DIR}/{CISCO_IOSXE_ZTP_CONFIG}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{iosxe_ztp_base64}")),
    };
    let iosv_ztp_base64 = base64_encode(&ztp_templates.cisco_iosv);
    let iosv_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{CISCO_ZTP_DIR}/{CISCO_IOSV_ZTP_CONFIG}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{iosv_ztp_base64}")),
    };

    let juniper_vjunos_ztp_base64 = base64_encode(&ztp_templates.juniper_vjunos);
    let juniper_vjunos_ztp_file = IgnitionFile {
        path: format!("/opt/ztp/{JUNIPER_ZTP_DIR}/{JUNIPER_ZTP_CONFIG}"),
        mode: 644,
        contents: IgnitionFileContents::new(&format!("data:;base64,{juniper_vjunos_ztp_base64}")),
    };

    let ignition_config = IgnitionConfig::new(
        vec![ignition_user],
        vec![
            sudo_config_file,
            hostname_file,
            arista_ztp_file,
            aruba_ztp_file,
            cumulus_ztp_file,
            iosxe_ztp_file,
            iosv_ztp_file,
            juniper_vjunos_ztp_file,
        ],
        vec![],
        vec![
            unit_webdir,
            unit_tftp,
            // srlinux_unit
        ],
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

    let device_disks: Vec<DeviceDisk> = vec![DeviceDisk {
        disk_device: DiskDevices::File,
        driver_name: DiskDrivers::Qemu,
        driver_format: DiskFormats::Qcow2,
        src_file: dst_boot_disk.clone(),
        target_dev: DiskTargets::target(&DiskBuses::Sata, 0)?,
        target_bus: DiskBuses::Sata,
    }];

    let domain = DomainTemplate {
        qemu_bin: config.qemu_bin.clone(),
        name: boot_server_name.to_owned(),
        memory: 512,
        cpu_architecture: CpuArchitecture::default(),
        cpu_model: CpuModels::default(),
        machine_type: MachineTypes::default(),
        cpu_count: 1,
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
