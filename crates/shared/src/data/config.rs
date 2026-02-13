use ipnet::Ipv4Net;
use serde_derive::{Deserialize, Serialize};

use super::container::ContainerImage;
// use super::node::NodeConfig;
use super::provider::VmProviders;

use crate::konst::{SHERPAD_HOST, SHERPAD_PORT, SHERPA_PASSWORD, SHERPA_USERNAME};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ZtpServer {
    pub enable: bool,
    pub username: Option<String>,
    pub password: Option<String>,
}
impl Default for ZtpServer {
    fn default() -> Self {
        Self {
            enable: true,
            username: Some(SHERPA_USERNAME.to_owned()),
            password: Some(SHERPA_PASSWORD.to_owned()),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ConfigurationManagement {
    #[serde(default)]
    pub ansible: bool,
    #[serde(default)]
    pub pyats: bool,
    #[serde(default)]
    pub nornir: bool,
}

/// Server connection configuration for WebSocket RPC
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ServerConnection {
    /// WebSocket URL (e.g., ws://localhost:3030/ws)
    pub url: Option<String>,
    /// Connection timeout in seconds
    pub timeout_secs: u64,
}

impl Default for ServerConnection {
    fn default() -> Self {
        Self {
            url: Some(format!("ws://{}:{}/ws", SHERPAD_HOST, SHERPAD_PORT)),
            timeout_secs: 3,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub name: String,
    pub vm_provider: VmProviders,
    pub qemu_bin: String,
    pub management_prefix_ipv4: Ipv4Net,
    pub images_dir: String,
    pub containers_dir: String,
    pub bins_dir: String,
    pub ztp_server: ZtpServer,
    pub configuration_management: ConfigurationManagement,
    pub container_images: Vec<ContainerImage>,
    #[serde(default)]
    pub server_connection: ServerConnection,
}

#[derive(Clone, Debug)]
pub struct Sherpa {
    pub base_dir: String,
    pub config_dir: String,
    pub config_file_path: String,
    pub ssh_dir: String,
    pub images_dir: String,
    pub containers_dir: String,
    pub bins_dir: String,
}
