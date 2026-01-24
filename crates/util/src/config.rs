use std::fs;
use std::path::Path;
use std::str::FromStr;

use anyhow::Result;
use ipnet::Ipv4Net;

use super::file_system::{create_file, expand_path};
use data::{Config, ContainerImage, InventoryManagement, NodeConfig, VmProviders, ZtpServer};
use konst::{
    QEMU_BIN, SHERPA_BASE_DIR, SHERPA_BINS_DIR, SHERPA_CONFIG_FILE, SHERPA_CONTAINERS_DIR,
    SHERPA_IMAGES_DIR, SHERPA_MANAGEMENT_NETWORK_IPV4, SHERPA_PASSWORD, SHERPA_USERNAME,
};

pub fn create_config(config: &Config, path: &str) -> Result<()> {
    let toml_string = toml::to_string_pretty(&config)?;
    create_file(path, toml_string)?;
    Ok(())
}
pub fn load_config(file_path: &str) -> Result<Config> {
    let expanded_path = shellexpand::tilde(file_path);
    let config_path = Path::new(expanded_path.as_ref());

    let contents = fs::read_to_string(config_path)?;
    let config: Config = toml::from_str(&contents)?;
    Ok(config)
}

pub fn default_config() -> Config {
    let container_images: Vec<ContainerImage> =
        vec![ContainerImage::dnsmasq(), ContainerImage::webdir()];
    let device_models: Vec<NodeConfig> = vec![
        NodeConfig::arista_veos(),
        NodeConfig::arista_ceos(),
        NodeConfig::aruba_aoscx(),
        NodeConfig::cisco_asav(),
        NodeConfig::cisco_cat8000v(),
        NodeConfig::cisco_cat9000v(),
        NodeConfig::cisco_csr1000v(),
        NodeConfig::cisco_iosxrv9000(),
        NodeConfig::cisco_nexus9300v(),
        NodeConfig::cisco_iosv(),
        NodeConfig::cisco_iosvl2(),
        NodeConfig::cisco_ise(),
        NodeConfig::juniper_vrouter(),
        NodeConfig::juniper_vswitch(),
        NodeConfig::juniper_vevolved(),
        NodeConfig::juniper_vsrxv3(),
        NodeConfig::nokia_srlinux(),
        NodeConfig::cumulus_linux(),
        NodeConfig::sonic_linux(),
        NodeConfig::alpine_linux(),
        NodeConfig::alma_linux(),
        NodeConfig::rocky_linux(),
        NodeConfig::centos_linux(),
        NodeConfig::fedora_linux(),
        NodeConfig::redhat_linux(),
        NodeConfig::ubuntu_linux(),
        NodeConfig::opensuse_linux(),
        NodeConfig::suse_linux(),
        NodeConfig::flatcar_linux(),
        NodeConfig::free_bsd(),
        NodeConfig::open_bsd(),
        NodeConfig::windows_server(),
        // Containers
        NodeConfig::surreal_db(),
        NodeConfig::mysql_db(),
        NodeConfig::postgresql_db(),
    ];
    // TODO: FIXME DEFAULT SHERPA MGMT
    let mgmt_prefix_ipv4 =
        Ipv4Net::from_str(SHERPA_MANAGEMENT_NETWORK_IPV4).expect("Failed to parse IPv4 network");

    let ztp_server = ZtpServer {
        enable: false,
        username: Some(SHERPA_USERNAME.to_owned()),
        password: Some(SHERPA_PASSWORD.to_owned()),
    };

    let boxes_dir = expand_path(&format!("{SHERPA_BASE_DIR}/{SHERPA_IMAGES_DIR}"));
    let containers_dir = expand_path(&format!("{SHERPA_BASE_DIR}/{SHERPA_CONTAINERS_DIR}"));
    let bins_dir = expand_path(&format!("{SHERPA_BASE_DIR}/{SHERPA_BINS_DIR}"));

    Config {
        name: SHERPA_CONFIG_FILE.to_owned(),
        vm_provider: VmProviders::default(),
        qemu_bin: QEMU_BIN.to_owned(),
        images_dir: boxes_dir,
        containers_dir,
        bins_dir,
        node_config: device_models,
        container_images,
        management_prefix_ipv4: mgmt_prefix_ipv4,
        inventory_management: InventoryManagement::default(),
        ztp_server,
    }
}
