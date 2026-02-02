use anyhow::Result;
use askama::Template;

use container::{Docker, run_container};
use data::{ContainerNetworkAttachment, SherpaNetwork, User, ZtpRecord};
use konst::{
    CONTAINER_DNSMASQ_NAME, CONTAINER_DNSMASQ_REPO, DNSMASQ_CONFIG_FILE, DNSMASQ_DIR,
    DNSMASQ_LEASES_FILE, NODE_CONFIGS_DIR, SHERPA_BASE_DIR, SHERPA_LABS_DIR,
    SHERPA_MANAGEMENT_NETWORK_NAME, TFTP_DIR, ZTP_DIR,
};
use template::{DnsmasqTemplate, SonicLinuxUserTemplate};
use util::{create_dir, create_file, get_ipv4_addr, term_msg_underline};

pub fn create_ztp_files(
    mgmt_net: &SherpaNetwork,
    sherpa_user: &User,
    lab_id: &str,
    ztp_records: &[ZtpRecord],
) -> Result<()> {
    // Create ZTP files
    term_msg_underline("Creating ZTP configs");
    let lab_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_LABS_DIR}/{lab_id}");

    // Create directories
    let ztp_dir = format!("{lab_dir}/{ZTP_DIR}");
    let ztp_configs_dir = format!("{ztp_dir}/{NODE_CONFIGS_DIR}");
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

    // Sonic
    let sonic_user_template = SonicLinuxUserTemplate {
        user: sherpa_user.clone(),
    };
    let sonic_user_rendered_template = sonic_user_template.render()?;
    create_file(
        &format!("{ztp_configs_dir}/sonic_ztp_user.sh"),
        sonic_user_rendered_template.clone(),
    )?;

    Ok(())
}

pub async fn create_boot_containers(
    docker_conn: &Docker,
    mgmt_net: &SherpaNetwork,
    lab_id: &str,
) -> Result<()> {
    // Setup directories for volume mounts
    let lab_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_LABS_DIR}/{lab_id}");
    let ztp_dir = format!("{lab_dir}/{ZTP_DIR}");

    // Volume mount dirs
    let dnsmasq_dir = format!("{ztp_dir}/{DNSMASQ_DIR}");
    let tftp_dir = format!("{ztp_dir}/{TFTP_DIR}");
    let configs_dir = format!("{ztp_dir}/{NODE_CONFIGS_DIR}");

    // Ensure directories exist
    create_dir(&dnsmasq_dir)?;
    create_dir(&tftp_dir)?;
    create_dir(&configs_dir)?;

    let dnsmasq_env_dns1 = format!("DNS1={}", mgmt_net.v4.first);
    let dnsmasq_env_dns2 = "DNS2=".to_string();
    let boot_server_ipv4 = mgmt_net.v4.boot_server.to_string();

    // Webdir service
    let webdir_config_volume = format!("{configs_dir}:/opt/{ZTP_DIR}/{NODE_CONFIGS_DIR}");

    // Dnsmasq service
    let dnsmasq_env_vars = vec![dnsmasq_env_dns1, dnsmasq_env_dns2];
    let dnsmasq_config_volume =
        format!("{dnsmasq_dir}/{DNSMASQ_CONFIG_FILE}:/etc/{DNSMASQ_CONFIG_FILE}");
    let dnsmasq_tftp_volume = format!("{tftp_dir}:/opt/{ZTP_DIR}/{TFTP_DIR}");

    let dnsmasq_volumes = vec![
        dnsmasq_config_volume,
        dnsmasq_tftp_volume,
        webdir_config_volume,
    ];
    let dnsmasq_capabilities = vec!["NET_ADMIN"];

    let management_network = ContainerNetworkAttachment {
        name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
        ipv4_address: Some(boot_server_ipv4),
    };

    run_container(
        docker_conn,
        &format!("{CONTAINER_DNSMASQ_NAME}-{lab_id}"),
        CONTAINER_DNSMASQ_REPO,
        dnsmasq_env_vars,
        dnsmasq_volumes,
        dnsmasq_capabilities,
        management_network,
        vec![],
        vec![],
        false,
    )
    .await?;

    Ok(())
}
