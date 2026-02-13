use std::fs;
use std::path::Path;
use std::str::FromStr;

use anyhow::Result;
use ipnet::Ipv4Net;

use super::file_system::{create_file, expand_path};
use crate::data::{
    Config, ConfigurationManagement, ContainerImage, ServerConnection, VmProviders, ZtpServer,
};
use crate::konst::{
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
        container_images,
        management_prefix_ipv4: mgmt_prefix_ipv4,
        configuration_management: ConfigurationManagement::default(),
        ztp_server,
        server_connection: ServerConnection::default(),
    }
}
