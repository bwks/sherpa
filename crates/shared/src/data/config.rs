use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;

use ipnet::{Ipv4Net, Ipv6Net};
use serde_derive::{Deserialize, Serialize};
use tracing::instrument;

use super::container::ContainerImage;
use super::provider::VmProviders;

use crate::konst::{
    HOST_MEMORY_DEFAULT_OVERCOMMIT_PERCENT, HOST_MEMORY_DEFAULT_RESERVE_MIB, OTEL_DEFAULT_ENDPOINT,
    OTEL_DEFAULT_SAMPLE_RATIO, OTEL_DEFAULT_SERVICE_NAME, SHERPA_MANAGEMENT_NETWORK_IPV4,
    SHERPA_SERVER_HTTP_PORT, SHERPA_SERVER_WS_PORT,
};
use crate::util::path_to_string;

/// Server connection configuration for WebSocket RPC. A field left out of the config file
/// takes its value from `ServerConnection::default()`.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct ServerConnection {
    pub url: Option<String>,
    pub http_port: u16,
    pub timeout_secs: u64,
    pub validate_certs: bool,
    pub ca_cert_path: Option<String>,
    pub insecure: bool,
}

impl Default for ServerConnection {
    fn default() -> Self {
        Self {
            url: None,
            http_port: SHERPA_SERVER_HTTP_PORT,
            timeout_secs: 3,
            validate_certs: true,
            ca_cert_path: None,
            insecure: false,
        }
    }
}

/// TLS configuration for server. A field left out of the config file takes its value from
/// `TlsConfig::default()`, so a `[tls]` section that sets only `san` keeps TLS on.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct TlsConfig {
    pub enabled: bool,

    pub cert_path: Option<String>,

    pub key_path: Option<String>,

    pub self_generate_cert: bool,

    pub cert_validity_days: u32,

    pub san: Vec<String>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cert_path: None,
            key_path: None,
            self_generate_cert: true,
            cert_validity_days: 365,
            san: vec![],
        }
    }
}

/// OpenTelemetry configuration.
/// When the `[otel]` section is absent from sherpa.toml, all fields take their
/// defaults from the constants in `shared::konst`.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct OtelConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub service_name: String,
    pub sample_ratio: f64,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: OTEL_DEFAULT_ENDPOINT.to_owned(),
            service_name: OTEL_DEFAULT_SERVICE_NAME.to_owned(),
            sample_ratio: OTEL_DEFAULT_SAMPLE_RATIO,
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
            server_connection: ServerConnection::default(),
            tls: TlsConfig::default(),
        }
    }
}

/// Scanner service configuration for periodic node/lab state reconciliation.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct ScannerConfig {
    pub enabled: bool,
    pub interval_secs: u64,
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 30,
        }
    }
}

/// Server-side Tailscale gateway settings, overridable in `sherpa.toml`.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct TailscaleGatewaySettings {
    pub image: String,
    pub socket_path: String,
    pub exec_timeout_secs: u32,
    pub daemon_ready_timeout_secs: u32,
    pub enrollment_timeout_secs: u32,
    pub connection_timeout_secs: u32,
    pub stop_timeout_secs: u32,
}

impl Default for TailscaleGatewaySettings {
    #[instrument(level = "debug")]
    fn default() -> Self {
        Self {
            image: "tailscale/tailscale:v1.102.3".to_owned(),
            socket_path: "/run/tailscale/tailscaled.sock".to_owned(),
            exec_timeout_secs: 75,
            daemon_ready_timeout_secs: 20,
            enrollment_timeout_secs: 60,
            connection_timeout_secs: 30,
            stop_timeout_secs: 10,
        }
    }
}

/// How much memory the server may give its VMs and unikernels: `[host_memory]` in
/// `sherpa.toml`. Nodes that would take more are refused when they are created or started.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HostMemorySettings {
    pub reserve_mib: u32,
    pub overcommit_percent: u32,
}

impl Default for HostMemorySettings {
    fn default() -> Self {
        Self {
            reserve_mib: HOST_MEMORY_DEFAULT_RESERVE_MIB,
            overcommit_percent: HOST_MEMORY_DEFAULT_OVERCOMMIT_PERCENT,
        }
    }
}

/// Full server configuration with defaults for optional sections.
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
    pub allow_signup: bool,
    #[serde(default)]
    pub container_images: Vec<ContainerImage>,
    #[serde(default)]
    pub server_connection: ServerConnection,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub otel: OtelConfig,
    #[serde(default)]
    pub scanner: ScannerConfig,
    #[serde(default)]
    pub tailscale: TailscaleGatewaySettings,
    #[serde(default)]
    pub host_memory: HostMemorySettings,
}

fn default_server_ipv4() -> Ipv4Addr {
    Ipv4Addr::LOCALHOST
}

fn default_ws_port() -> u16 {
    SHERPA_SERVER_WS_PORT
}

fn default_http_port() -> u16 {
    SHERPA_SERVER_HTTP_PORT
}

fn default_management_prefix() -> Ipv4Net {
    SHERPA_MANAGEMENT_NETWORK_IPV4
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
    #[must_use]
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
    use crate::util::default_config;

    #[test]
    fn server_config_005_a_config_without_otel_scanner_or_host_memory_loads_with_their_defaults() {
        let mut value = toml::Value::try_from(default_config()).unwrap();
        let table = value.as_table_mut().unwrap();
        for name in ["otel", "scanner", "host_memory"] {
            table.remove(name);
        }
        let config: Config = toml::from_str(&toml::to_string(&value).unwrap()).unwrap();
        assert_eq!(
            toml::to_string(&config.otel).unwrap(),
            toml::to_string(&OtelConfig::default()).unwrap()
        );
        assert_eq!(
            toml::to_string(&config.scanner).unwrap(),
            toml::to_string(&ScannerConfig::default()).unwrap()
        );
        assert_eq!(config.host_memory, HostMemorySettings::default());

        let otel: OtelConfig = toml::from_str("enabled = true").unwrap();
        let default_otel = OtelConfig::default();
        assert!(otel.enabled);
        assert_eq!(otel.endpoint, default_otel.endpoint);
        assert_eq!(otel.service_name, default_otel.service_name);

        let scanner: ScannerConfig = toml::from_str("enabled = false").unwrap();
        assert!(!scanner.enabled);
        assert_eq!(
            scanner.interval_secs,
            ScannerConfig::default().interval_secs
        );

        let memory: HostMemorySettings = toml::from_str("overcommit_percent = 150").unwrap();
        assert_eq!(memory.overcommit_percent, 150);
        assert_eq!(
            memory.reserve_mib,
            HostMemorySettings::default().reserve_mib
        );
    }

    #[test]
    fn server_config_006_a_key_host_memory_does_not_have_is_refused() {
        assert!(toml::from_str::<HostMemorySettings>("reserve = 1").is_err());
    }

    #[test]
    fn client_config_004_the_certificate_port_is_the_one_in_server_connection() {
        let config: ClientConfig =
            toml::from_str("http_port = 9000\n[server_connection]\nhttp_port = 8443\n")
                .expect("parses");
        assert_eq!(config.server_connection.http_port, 8443);
    }

    #[test]
    fn server_config_004_an_otel_section_from_an_older_release_still_loads() {
        let otel: OtelConfig = toml::from_str(
            r#"
            enabled = true
            protocol = "grpc"
            "#,
        )
        .expect("parses");
        assert!(otel.enabled);
        let written = toml::to_string_pretty(&otel).expect("serializes");
        assert!(!written.contains("protocol"), "{written}");
    }
}
