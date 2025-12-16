use std::path;

use anyhow::Result;
use askama::Template;

use container::{Docker, run_container};
use data::{ContainerNetworkAttachment, Dns, SherpaNetwork, User, ZtpRecord, ZtpTemplates};
use konst::{
    ARISTA_VEOS_ZTP_SCRIPT, ARISTA_ZTP_DIR, ARUBA_ZTP_CONFIG, ARUBA_ZTP_DIR, CISCO_ZTP_DIR,
    CONTAINER_DNSMASQ_NAME, CONTAINER_DNSMASQ_REPO, CUMULUS_ZTP_CONFIG, CUMULUS_ZTP_DIR,
    DEVICE_CONFIGS_DIR, DNSMASQ_CONFIG_FILE, DNSMASQ_DIR, DNSMASQ_LEASES_FILE, JUNIPER_ZTP_DIR,
    SHERPA_MANAGEMENT_NETWORK_NAME, TEMP_DIR, TFTP_DIR, ZTP_DIR,
};
use template::{
    ArubaAoscxTemplate, CumulusLinuxZtpTemplate, DnsmasqTemplate, SonicLinuxUserTemplate,
    arista_veos_ztp_script,
};
use util::{create_dir, create_file, get_ipv4_addr, pub_ssh_key_to_md5_hash, term_msg_underline};

pub fn create_ztp_files(
    mgmt_net: &SherpaNetwork,
    sherpa_user: &User,
    dns: &Dns,
    ztp_records: &[ZtpRecord],
) -> Result<ZtpTemplates> {
    // Create ZTP files
    term_msg_underline("Creating ZTP configs");

    // Create directories
    let ztp_dir = format!("{TEMP_DIR}/{ZTP_DIR}");
    let ztp_configs_dir = format!("{ztp_dir}/{DEVICE_CONFIGS_DIR}");
    let dnsmasq_dir = format!("{ztp_dir}/{DNSMASQ_DIR}");
    create_dir(&ztp_dir)?;
    create_dir(&ztp_configs_dir)?;
    create_dir(&dnsmasq_dir)?;

    let dnsmaq_template = DnsmasqTemplate {
        tftp_server_ipv4: mgmt_net.v4.boot_server.to_string(),
        gateway_ipv4: mgmt_net.v4.first.to_string(),
        dhcp_start: get_ipv4_addr(&mgmt_net.v4.prefix, 10)?.to_string(),
        dhcp_end: get_ipv4_addr(&mgmt_net.v4.prefix, 254)?.to_string(),
        ztp_records: ztp_records.to_vec().clone(),
    };

    let dnsmasq_rendered_template = dnsmaq_template.render()?;
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

    // Sonic
    let sonic_user_template = SonicLinuxUserTemplate {
        user: sherpa_user.clone(),
    };
    let sonic_user_rendered_template = sonic_user_template.render()?;
    create_file(
        &format!("{ztp_configs_dir}/sonic_ztp_user.sh"),
        sonic_user_rendered_template.clone(),
    )?;

    // Cisco
    let cisco_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{CISCO_ZTP_DIR}");
    create_dir(&cisco_dir)?;
    let mut cisco_user = sherpa_user.clone();
    cisco_user.ssh_public_key.key = pub_ssh_key_to_md5_hash(&cisco_user.ssh_public_key.key)?;

    // Juniper vevolved
    let juniper_dir = format!("{TEMP_DIR}/{ZTP_DIR}/{JUNIPER_ZTP_DIR}");
    create_dir(&juniper_dir)?;

    Ok(ZtpTemplates {
        arista_eos: arista_ztp_script,
        aruba_aos: aruba_rendered_template,
        cumulus_linux: cumulus_rendered_template,
    })
}

pub async fn create_boot_containers(
    docker_conn: &Docker,
    mgmt_net: &SherpaNetwork,
    lab_id: &str,
) -> Result<()> {
    let project_path = path::absolute(".")?;
    let project_dir = project_path.display();
    let ztp_dir = format!("{TEMP_DIR}/{ZTP_DIR}");
    let dnsmasq_dir = format!("{ztp_dir}/{DNSMASQ_DIR}");
    let tftp_dir = format!("{ztp_dir}/{TFTP_DIR}");
    let configs_dir = format!("{ztp_dir}/{DEVICE_CONFIGS_DIR}");
    let dnsmasq_env_dns1 = format!("DNS1={}", mgmt_net.v4.first);
    let dnsmasq_env_dns2 = "DNS2=";
    let boot_server_ipv4 = mgmt_net.v4.boot_server.to_string();

    // Webdir
    let webdir_config_dir =
        format!("{project_dir}/{configs_dir}:/opt/{ZTP_DIR}/{DEVICE_CONFIGS_DIR}");

    // Dnsmasq
    let dnsmasq_env_vars = vec![dnsmasq_env_dns1.as_str(), dnsmasq_env_dns2];
    let dnsmasq_config = format!(
        "{}/{dnsmasq_dir}/{DNSMASQ_CONFIG_FILE}:/etc/{DNSMASQ_CONFIG_FILE}",
        project_dir
    );
    let dnsmasq_tftp = format!("{}/{tftp_dir}:/opt/{ZTP_DIR}/{TFTP_DIR}", project_dir);

    let dnsmasq_volumes = vec![
        dnsmasq_config.as_str(),
        dnsmasq_tftp.as_str(),
        webdir_config_dir.as_str(),
    ];
    let dnsmasq_capabilities = vec!["NET_ADMIN"];

    let network_attachments = vec![ContainerNetworkAttachment {
        name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
        ipv4_address: Some(boot_server_ipv4),
    }];

    run_container(
        docker_conn,
        &format!("{}-{}", CONTAINER_DNSMASQ_NAME, lab_id),
        CONTAINER_DNSMASQ_REPO,
        dnsmasq_env_vars,
        dnsmasq_volumes,
        dnsmasq_capabilities,
        network_attachments,
    )
    .await?;

    Ok(())
}
