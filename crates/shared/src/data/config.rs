use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;

use ipnet::{Ipv4Net, Ipv6Net};
use serde_derive::{Deserialize, Serialize};

use super::container::ContainerImage;
// use super::node::NodeConfig;
use super::provider::VmProviders;

use crate::konst::{
    SHERPA_PASSWORD, SHERPA_SERVER_HTTP_PORT, SHERPA_SERVER_WS_PORT, SHERPA_USERNAME,
};
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
    #[serde(default)]
    pub server_ipv6: Option<Ipv6Addr>,
    #[serde(default = "default_ws_port")]
    pub ws_port: u16,
    #[serde(default = "default_http_port")]
    pub http_port: u16,
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
            server_ipv6: None,
            ws_port: default_ws_port(),
            http_port: default_http_port(),
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
    #[serde(default)]
    pub server_ipv6: Option<Ipv6Addr>,
    #[serde(default = "default_ws_port")]
    pub ws_port: u16,
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    pub vm_provider: VmProviders,
    pub qemu_bin: String,
    #[serde(default = "default_management_prefix")]
    pub management_prefix_ipv4: Ipv4Net,
    #[serde(default)]
    pub management_prefix_ipv6: Option<Ipv6Net>,
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

fn default_ws_port() -> u16 {
    SHERPA_SERVER_WS_PORT
}

fn default_http_port() -> u16 {
    SHERPA_SERVER_HTTP_PORT
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ztp_server_default() {
        let ztp = ZtpServer::default();
        assert_eq!(ztp.enable, true);
        assert_eq!(ztp.username, Some(SHERPA_USERNAME.to_owned()));
        assert_eq!(ztp.password, Some(SHERPA_PASSWORD.to_owned()));
    }

    #[test]
    fn test_ztp_server_serde_roundtrip() {
        let ztp = ZtpServer {
            enable: false,
            username: Some("test".to_string()),
            password: None,
        };
        let json = serde_json::to_string(&ztp).expect("serializes");
        let back: ZtpServer = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.enable, false);
        assert_eq!(back.username, Some("test".to_string()));
        assert!(back.password.is_none());
    }

    #[test]
    fn test_configuration_management_default() {
        let cm = ConfigurationManagement::default();
        assert_eq!(cm.ansible, false);
        assert_eq!(cm.pyats, false);
        assert_eq!(cm.nornir, false);
    }

    #[test]
    fn test_server_connection_default() {
        let sc = ServerConnection::default();
        assert!(sc.url.is_none());
        assert_eq!(sc.timeout_secs, 3);
        assert_eq!(sc.validate_certs, true);
        assert!(sc.ca_cert_path.is_none());
        assert_eq!(sc.insecure, false);
    }

    #[test]
    fn test_tls_config_default() {
        let tls = TlsConfig::default();
        assert_eq!(tls.enabled, true);
        assert!(tls.cert_path.is_none());
        assert!(tls.key_path.is_none());
        assert_eq!(tls.auto_generate_cert, true);
        assert_eq!(tls.cert_validity_days, 365);
        assert!(tls.san.is_empty());
    }

    #[test]
    fn test_client_config_default() {
        let config = ClientConfig::default();
        assert_eq!(config.server_ipv4, Ipv4Addr::new(127, 0, 0, 1));
        assert_eq!(config.ws_port, SHERPA_SERVER_WS_PORT);
        assert_eq!(config.http_port, SHERPA_SERVER_HTTP_PORT);
        assert!(config.server_ipv6.is_none());
    }

    #[test]
    fn test_client_config_serde_roundtrip() {
        let config = ClientConfig::default();
        let toml_str = toml::to_string_pretty(&config).expect("serializes");
        let back: ClientConfig = toml::from_str(&toml_str).expect("deserializes");
        assert_eq!(back.server_ipv4, config.server_ipv4);
        assert_eq!(back.ws_port, config.ws_port);
        assert_eq!(back.http_port, config.http_port);
    }
}
