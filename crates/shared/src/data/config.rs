use std::net::Ipv4Addr;
use std::path::Path;

use ipnet::Ipv4Net;
use serde_derive::{Deserialize, Serialize};

use super::container::ContainerImage;
// use super::node::NodeConfig;
use super::provider::VmProviders;

use crate::konst::{SHERPA_PASSWORD, SHERPA_USERNAME};
use crate::util::path_to_string;

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
    #[serde(default)]
    pub url: Option<String>,
    /// Connection timeout in seconds
    #[serde(default)]
    pub timeout_secs: u64,
    /// Validate server TLS certificates against system CA store
    #[serde(default)]
    pub validate_certs: bool,
    /// Path to custom CA certificate for validating server cert
    /// Use this for self-signed certificates
    #[serde(default)]
    pub ca_cert_path: Option<String>,
    /// Allow insecure connections (skip cert validation)
    /// DANGEROUS: Only for development/testing
    #[serde(default)]
    pub insecure: bool,
}

impl Default for ServerConnection {
    fn default() -> Self {
        Self {
            url: None,
            timeout_secs: 3,
            validate_certs: true,
            ca_cert_path: None,
            insecure: false,
        }
    }
}

/// TLS configuration for server
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TlsConfig {
    /// Enable TLS for WebSocket connections
    #[serde(default)]
    pub enabled: bool,

    /// Path to server certificate (PEM format)
    /// If not provided, uses /opt/sherpa/.certs/server.crt
    #[serde(default)]
    pub cert_path: Option<String>,

    /// Path to server private key (PEM format)
    /// If not provided, uses /opt/sherpa/.certs/server.key
    #[serde(default)]
    pub key_path: Option<String>,

    /// Auto-generate self-signed certificate if none exists
    #[serde(default)]
    pub auto_generate_cert: bool,

    /// Certificate validity in days (for auto-generated certs)
    #[serde(default)]
    pub cert_validity_days: u32,

    /// Subject Alternative Names for certificate (e.g., DNS names, IPs)
    #[serde(default)]
    pub san: Vec<String>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cert_path: None,
            key_path: None,
            auto_generate_cert: true,
            cert_validity_days: 365,
            san: vec![],
        }
    }
}

/// Client-only configuration for connecting to a Sherpa server.
/// All fields have sensible defaults so a minimal TOML works.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_server_ipv4")]
    pub server_ipv4: Ipv4Addr,
    #[serde(default = "default_server_port")]
    pub server_port: u16,
    #[serde(default)]
    pub server_connection: ServerConnection,
    #[serde(default)]
    pub tls: TlsConfig,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            server_ipv4: default_server_ipv4(),
            server_port: default_server_port(),
            server_connection: ServerConnection::default(),
            tls: TlsConfig::default(),
        }
    }
}

/// Full server configuration. All server-specific fields are required.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub name: String,
    #[serde(default = "default_server_ipv4")]
    pub server_ipv4: Ipv4Addr,
    #[serde(default = "default_server_port")]
    pub server_port: u16,
    pub vm_provider: VmProviders,
    pub qemu_bin: String,
    #[serde(default = "default_management_prefix")]
    pub management_prefix_ipv4: Ipv4Net,
    pub images_dir: String,
    pub containers_dir: String,
    pub bins_dir: String,
    #[serde(default)]
    pub ztp_server: ZtpServer,
    #[serde(default)]
    pub configuration_management: ConfigurationManagement,
    #[serde(default)]
    pub container_images: Vec<ContainerImage>,
    #[serde(default)]
    pub server_connection: ServerConnection,
    #[serde(default)]
    pub tls: TlsConfig,
}

fn default_server_ipv4() -> Ipv4Addr {
    Ipv4Addr::new(127, 0, 0, 1)
}

fn default_server_port() -> u16 {
    3030
}

fn default_management_prefix() -> Ipv4Net {
    use std::str::FromStr;
    Ipv4Net::from_str(crate::konst::SHERPA_MANAGEMENT_NETWORK_IPV4)
        .expect("Failed to parse default management network prefix")
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

impl Sherpa {
    /// Build a `Sherpa` with all paths derived from a base directory.
    /// Uses platform-native path separators.
    pub fn from_base_dir(base_dir: String) -> Self {
        let base = Path::new(&base_dir);
        let config_dir = base.join(crate::konst::SHERPA_CONFIG_DIR);
        Self {
            config_file_path: path_to_string(&config_dir.join(crate::konst::SHERPA_CONFIG_FILE)),
            ssh_dir: path_to_string(&base.join(crate::konst::SHERPA_SSH_DIR)),
            images_dir: path_to_string(&base.join(crate::konst::SHERPA_IMAGES_DIR)),
            containers_dir: path_to_string(&base.join(crate::konst::SHERPA_CONTAINERS_DIR)),
            bins_dir: path_to_string(&base.join(crate::konst::SHERPA_BINS_DIR)),
            config_dir: path_to_string(&config_dir),
            base_dir,
        }
    }
}
