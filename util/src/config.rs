use anyhow::Result;

use super::{create_config, expand_path};

// impl Default for Config {
//     fn default() -> Self {
//         let container_images: Vec<ContainerImage> = vec![
//             ContainerImage::dnsmasq(),
//             ContainerImage::webdir(),
//             ContainerImage::srlinux(),
//         ];
//         let device_models: Vec<DeviceModel> = vec![
//             DeviceModel::arista_veos(),
//             DeviceModel::arista_ceos(),
//             DeviceModel::aruba_aoscx(),
//             DeviceModel::cisco_asav(),
//             DeviceModel::cisco_cat8000v(),
//             DeviceModel::cisco_cat9000v(),
//             DeviceModel::cisco_csr1000v(),
//             DeviceModel::cisco_iosxrv9000(),
//             DeviceModel::cisco_nexus9300v(),
//             DeviceModel::cisco_iosv(),
//             DeviceModel::cisco_iosvl2(),
//             DeviceModel::juniper_vrouter(),
//             DeviceModel::juniper_vswitch(),
//             DeviceModel::juniper_vevolved(),
//             DeviceModel::juniper_vsrxv3(),
//             DeviceModel::nokia_srlinux(),
//             DeviceModel::cumulus_linux(),
//             DeviceModel::centos_linux(),
//             DeviceModel::fedora_linux(),
//             DeviceModel::redhat_linux(),
//             DeviceModel::ubuntu_linux(),
//             DeviceModel::opensuse_linux(),
//             DeviceModel::suse_linux(),
//             DeviceModel::flatcar_linux(),
//         ];
//         let mgmt_prefix_ipv4 = Ipv4Network::from_str(SHERPA_MANAGEMENT_NETWORK_IPV4)
//             .expect("Failed to parse IPv4 network");

//         let ztp_server = ZtpServer {
//             enable: false,
//             username: Some(SHERPA_USERNAME.to_owned()),
//             password: Some(SHERPA_PASSWORD.to_owned()),
//         };

//         let boxes_dir = expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_BOXES_DIR}"));
//         let containers_dir = expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_CONTAINERS_DIR}"));
//         let bins_dir = expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_BINS_DIR}"));

//         Config {
//             name: SHERPA_CONFIG_FILE.to_owned(),
//             vm_provider: VmProviders::default(),
//             qemu_bin: QEMU_BIN.to_owned(),
//             boxes_dir,
//             containers_dir,
//             bins_dir,
//             device_models,
//             container_images,
//             management_prefix_ipv4: mgmt_prefix_ipv4,
//             inventory_management: InventoryManagement::default(),
//             ztp_server,
//         }
//     }
// }

// impl Config {
//     pub fn create(&self, path: &str) -> Result<()> {
//         let toml_string = toml::to_string_pretty(&self)?;
//         create_file(path, toml_string)?;
//         Ok(())
//     }
//     pub fn load(file_path: &str) -> Result<Config> {
//         let expanded_path = shellexpand::tilde(file_path);
//         let config_path = Path::new(expanded_path.as_ref());

//         let contents = fs::read_to_string(config_path)?;
//         let config: Config = toml::from_str(&contents)?;
//         Ok(config)
//     }
// }
// impl Default for Sherpa {
//     fn default() -> Self {
//         let config_dir = expand_path(SHERPA_CONFIG_DIR);
//         let boxes_dir = expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_BOXES_DIR}"));
//         let config_path = expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_CONFIG_FILE}"));
//         let containers_dir = expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_CONTAINERS_DIR}"));
//         let bins_dir = expand_path(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_BINS_DIR}"));
//         Self {
//             config_dir,
//             config_path,
//             boxes_dir,
//             containers_dir,
//             bins_dir,
//         }
//     }
// }

pub fn create_config() {}
