use ipnet::Ipv4Net;
use serde_derive::{Deserialize, Serialize};
use std::net::Ipv4Addr;

use super::container::ContainerImage;
// use super::node::NodeConfig;
use super::provider::VmProviders;

use crate::konst::{SHERPA_PASSWORD, SHERPA_USERNAME};

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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub name: String,
    pub server_ipv4: Ipv4Addr,
    pub server_port: u16,
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
    #[serde(default)]
    pub tls: TlsConfig,
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
