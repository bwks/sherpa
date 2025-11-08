use std::path;

use anyhow::Result;
use askama::Template;

use container::{Docker, run_container};
use data::{
    BiosTypes, CloneDisk, Config, ConnectionTypes, CpuArchitecture, CpuModels, DeviceDisk,
    DeviceModels, DiskBuses, DiskDevices, DiskDrivers, DiskFormats, DiskTargets, Dns, Interface,
    InterfaceTypes, MachineTypes, MgmtInterfaces, QemuCommand, Sherpa, SherpaNetwork, User,
    ZtpRecord, ZtpTemplates,
};
use konst::{
    ARISTA_VEOS_ZTP_SCRIPT, ARISTA_ZTP_DIR, ARUBA_ZTP_CONFIG, ARUBA_ZTP_DIR, BOOT_SERVER_MAC,
    BOOT_SERVER_NAME, CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_ZTP_CONFIG, CISCO_ZTP_DIR,
    CONTAINER_DISK_NAME, CONTAINER_DNSMASQ_NAME, CONTAINER_DNSMASQ_REPO, CONTAINER_WEBDIR_NAME,
    CONTAINER_WEBDIR_REPO, CUMULUS_ZTP_CONFIG, CUMULUS_ZTP_DIR, DEVICE_CONFIGS_DIR,
    DNSMASQ_CONFIG_FILE, DNSMASQ_DIR, DNSMASQ_LEASES_FILE, JUNIPER_ZTP_DIR, JUNIPER_ZTP_SCRIPT,
    MTU_JUMBO_INT, SHERPA_BLANK_DISK_DIR, SHERPA_BLANK_DISK_EXT4_500M,
    SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_PASSWORD_HASH, SHERPA_STORAGE_POOL_PATH,
    SHERPA_USERNAME, TELNET_PORT, TEMP_DIR, TFTP_DIR, ZTP_DIR, ZTP_JSON,
};
use template::{
    ArubaAoscxTemplate, BootServer, CiscoIosXeZtpTemplate, CiscoIosvZtpTemplate,
    Contents as IgnitionFileContents, CumulusLinuxZtpTemplate, DnsmasqTemplate, DomainTemplate,
    File as IgnitionFile, FileSystem as IgnitionFileSystem, IgnitionConfig, Link as IgnitionLink,
    Unit as IgnitionUnit, User as IgnitionUser, arista_veos_ztp_script,
    juniper_vevolved_ztp_script,
};
use util::{
    base64_encode, copy_file, copy_to_ext4_image, create_dir, create_file, get_ip, get_ipv4_addr,
    pub_ssh_key_to_md5_hash, term_msg_underline,
};

pub fn create_ztp_files(
    mgmt_net: &SherpaNetwork,
    sherpa_user: &User,
    dns: &Dns,
    ztp_records: &Vec<ZtpRecord>,
) -> Result<ZtpTemplates> {
    // Create ZTP files
    term_msg_underline("Creating ZTP configs");

    let ztp_dir = format!("{TEMP_DIR}/{ZTP_DIR}");
    let dnsmasq_dir = format!("{ztp_dir}/{DNSMASQ_DIR}");
    let configs_dir = format!("{ztp_dir}/{DEVICE_CONFIGS_DIR}");

    let dnsmaq_template = DnsmasqTemplate {
        tftp_server_ipv4: mgmt_net.v4.boot_server.to_string(),
        gateway_ipv4: mgmt_net.v4.first.to_string(),
        dhcp_start: get_ipv4_addr(mgmt_net.v4.prefix, 20)?.to_string(),
        dhcp_end: get_ipv4_addr(mgmt_net.v4.prefix, 254)?.to_string(),
        ztp_records: ztp_records.clone(),
    };

    let dnsmasq_rendered_template = dnsmaq_template.render()?;
    create_dir(&dnsmasq_dir)?;
    create_file(
        &format!("{dnsmasq_dir}/{DNSMASQ_CONFIG_FILE}"),
        dnsmasq_rendered_template,
    )?;
    create_file(
        &format!("{dnsmasq_dir}/{DNSMASQ_LEASES_FILE}"),
        "".to_string(),
    )?;

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

pub async fn create_boot_containers(docker_conn: &Docker, mgmt_net: &SherpaNetwork) -> Result<()> {
    let project_path = path::absolute(".")?;
    let project_dir = project_path.display();
    let ztp_dir = format!("{TEMP_DIR}/{ZTP_DIR}");
    let dnsmasq_dir = format!("{ztp_dir}/{DNSMASQ_DIR}");
    let tftp_dir = format!("{ztp_dir}/{TFTP_DIR}");
    let configs_dir = format!("{ztp_dir}/{DEVICE_CONFIGS_DIR}");
    let dnsmasq_env_dns1 = format!("DNS1={}", mgmt_net.v4.first);
    let boot_server_ipv4 = mgmt_net.v4.boot_server.to_string();
    let web_server_ipv4 = mgmt_net.v4.web_server.to_string();

    // Dnsmasq
    let dnsmasq_env_vars = vec![dnsmasq_env_dns1.as_str()];
    let dnsmasq_config = format!(
        "{}/{dnsmasq_dir}/{DNSMASQ_CONFIG_FILE}:/etc/{DNSMASQ_CONFIG_FILE}",
        project_dir
    );
    let dnsmasq_leases = format!(
        "{}/{dnsmasq_dir}/{DNSMASQ_LEASES_FILE}:/var/lib/misc/{DNSMASQ_LEASES_FILE}",
        project_dir
    );
    let dnsmasq_tftp = format!("{}/{tftp_dir}:/opt/{ZTP_DIR}/{TFTP_DIR}", project_dir);

    let dnsmasq_volumes = vec![
        dnsmasq_config.as_str(),
        dnsmasq_leases.as_str(),
        dnsmasq_tftp.as_str(),
    ];
    let dnsmasq_capabilities = vec!["NET_ADMIN"];

    run_container(
        &docker_conn,
        CONTAINER_DNSMASQ_NAME,
        CONTAINER_DNSMASQ_REPO,
        dnsmasq_env_vars,
        dnsmasq_volumes,
        dnsmasq_capabilities,
        SHERPA_MANAGEMENT_NETWORK_NAME,
        &boot_server_ipv4,
    )
    .await?;

    // Webdir
    let webdir_env_vars = vec![];
    let webdir_config_dir =
        format!("{project_dir}/{configs_dir}:/opt/{ZTP_DIR}/{DEVICE_CONFIGS_DIR}");
    let webdir_leases = format!(
        "{project_dir}/{dnsmasq_dir}/{DNSMASQ_LEASES_FILE}:/opt/{ZTP_DIR}/{DNSMASQ_DIR}/{DNSMASQ_LEASES_FILE}:ro",
    );
    let webdir_volumes = vec![webdir_config_dir.as_str(), webdir_leases.as_str()];
    let webdir_capabilities = vec![];

    run_container(
        &docker_conn,
        CONTAINER_WEBDIR_NAME,
        CONTAINER_WEBDIR_REPO,
        webdir_env_vars,
        webdir_volumes,
        webdir_capabilities,
        SHERPA_MANAGEMENT_NETWORK_NAME,
        &web_server_ipv4,
    )
    .await?;

    Ok(())
}
