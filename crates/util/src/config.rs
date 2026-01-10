use std::fs;
use std::path::Path;
use std::str::FromStr;

use anyhow::Result;
use ipnet::Ipv4Net;

use super::file_system::{create_file, expand_path};
use data::{Config, ContainerImage, InventoryManagement, NodeInstance, VmProviders, ZtpServer};
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
    let device_models: Vec<NodeInstance> = vec![
        NodeInstance::arista_veos(),
        NodeInstance::arista_ceos(),
        NodeInstance::aruba_aoscx(),
        NodeInstance::cisco_asav(),
        NodeInstance::cisco_cat8000v(),
        NodeInstance::cisco_cat9000v(),
        NodeInstance::cisco_csr1000v(),
        NodeInstance::cisco_iosxrv9000(),
        NodeInstance::cisco_nexus9300v(),
        NodeInstance::cisco_iosv(),
        NodeInstance::cisco_iosvl2(),
        NodeInstance::cisco_ise(),
        NodeInstance::juniper_vrouter(),
        NodeInstance::juniper_vswitch(),
        NodeInstance::juniper_vevolved(),
        NodeInstance::juniper_vsrxv3(),
        NodeInstance::nokia_srlinux(),
        NodeInstance::cumulus_linux(),
        NodeInstance::sonic_linux(),
        NodeInstance::alpine_linux(),
        NodeInstance::alma_linux(),
        NodeInstance::rocky_linux(),
        NodeInstance::centos_linux(),
        NodeInstance::fedora_linux(),
        NodeInstance::redhat_linux(),
        NodeInstance::ubuntu_linux(),
        NodeInstance::opensuse_linux(),
        NodeInstance::suse_linux(),
        NodeInstance::flatcar_linux(),
        NodeInstance::free_bsd(),
        NodeInstance::open_bsd(),
        NodeInstance::windows_server(),
        // Containers
        NodeInstance::surreal_db(),
        NodeInstance::mysql_db(),
        NodeInstance::postgresql_db(),
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
        device_models,
        container_images,
        management_prefix_ipv4: mgmt_prefix_ipv4,
        inventory_management: InventoryManagement::default(),
        ztp_server,
    }
}
