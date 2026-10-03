use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use ipnet::Ipv4Net;
use tracing::instrument;

use super::file_system::create_file;
use crate::data::{
    ClientConfig, Config, ConfigurationManagement, ContainerImage, OtelConfig, ScannerConfig,
    ServerConnection, TailscaleGatewaySettings, TlsConfig, VmProviders, ZtpServer,
};
use crate::konst::{
    QEMU_BIN, SHERPA_BINS_PATH, SHERPA_CONFIG_FILE, SHERPA_CONTAINERS_PATH, SHERPA_IMAGES_PATH,
    SHERPA_MANAGEMENT_NETWORK_IPV4, SHERPA_PASSWORD, SHERPA_SERVER_HTTP_PORT,
    SHERPA_SERVER_WS_PORT, SHERPA_USERNAME,
};

/// Build WebSocket URL from config
pub fn build_websocket_url(config: &Config) -> String {
    // Check if explicit URL is set
    if let Some(ref url) = config.server_connection.url {
        return url.clone();
    }

    // Construct URL based on TLS config
    let scheme = if config.tls.enabled { "wss" } else { "ws" };
    let host = config.server_ipv4;
    let port = config.ws_port;

    format!("{}://{}:{}/ws", scheme, host, port)
}

#[instrument(skip(config), level = "debug")]
pub fn create_config(config: &Config, path: &str) -> Result<()> {
    let toml_string =
        toml::to_string_pretty(config).context("Unable to serialize server config")?;
    create_file(path, toml_string)?;
    Ok(())
}
#[instrument(level = "debug")]
pub fn load_config(file_path: &str) -> Result<Config> {
    let expanded_path = shellexpand::tilde(file_path);
    let config_path = Path::new(expanded_path.as_ref());

    let contents = fs::read_to_string(config_path)
        .with_context(|| format!("Unable to read server config: {}", config_path.display()))?;
    toml::from_str(&contents).context("Invalid server configuration")
}

/// Build WebSocket URL from client config
pub fn build_client_websocket_url(config: &ClientConfig) -> String {
    if let Some(ref url) = config.server_connection.url {
        return url.clone();
    }

    let scheme = if config.tls.enabled { "wss" } else { "ws" };
    let host = config.server_ipv4;
    let port = config.ws_port;

    format!("{}://{}:{}/ws", scheme, host, port)
}

pub fn create_client_config(config: &ClientConfig, path: &str) -> Result<()> {
    let toml_string = toml::to_string_pretty(&config)?;
    create_file(path, toml_string)?;
    Ok(())
}

pub fn load_client_config(file_path: &str) -> Result<ClientConfig> {
    let expanded_path = shellexpand::tilde(file_path);
    let config_path = Path::new(expanded_path.as_ref());

    let contents = fs::read_to_string(config_path)?;
    let config: ClientConfig = toml::from_str(&contents)?;
    Ok(config)
}

#[instrument(level = "debug")]
pub fn default_config() -> Config {
    let container_images: Vec<ContainerImage> =
        vec![ContainerImage::dnsmasq(), ContainerImage::webdir()];

    // SAFETY: This is a compile-time constant, so parsing cannot fail.
    #[allow(clippy::expect_used)]
    let mgmt_prefix_ipv4 =
        Ipv4Net::from_str(SHERPA_MANAGEMENT_NETWORK_IPV4).expect("Failed to parse IPv4 network");

    let ztp_server = ZtpServer {
        enable: false,
        username: Some(SHERPA_USERNAME.to_owned()),
        password: Some(SHERPA_PASSWORD.to_owned()),
    };

    let boxes_dir = SHERPA_IMAGES_PATH.to_owned();
    let containers_dir = SHERPA_CONTAINERS_PATH.to_owned();
    let bins_dir = SHERPA_BINS_PATH.to_owned();

    Config {
        name: SHERPA_CONFIG_FILE.to_owned(),
        vm_provider: VmProviders::default(),
        qemu_bin: QEMU_BIN.to_owned(),
        images_dir: boxes_dir,
        containers_dir,
        bins_dir,
        container_images,
        management_prefix_ipv4: mgmt_prefix_ipv4,
        management_prefix_ipv6: None,
        configuration_management: ConfigurationManagement::default(),
        ztp_server,
        server_connection: ServerConnection::default(),
        server_ipv4: Ipv4Addr::new(127, 0, 0, 1),
        server_ipv6: None,
        ws_port: SHERPA_SERVER_WS_PORT,
        http_port: SHERPA_SERVER_HTTP_PORT,
        tls: TlsConfig::default(),
        otel: OtelConfig::default(),
        scanner: ScannerConfig::default(),
        tailscale: TailscaleGatewaySettings::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Sherpa;

    fn tailscale_defaults() -> toml::Value {
        toml::Value::try_from(TailscaleGatewaySettings::default()).unwrap()
    }

    fn load_server_fixture(extra: &str) -> Result<Config> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("sherpa.toml");
        let mut config = toml::Value::try_from(default_config())?;
        config.as_table_mut().unwrap().remove("tailscale");
        fs::write(&path, format!("{}\n{extra}", toml::to_string(&config)?))?;
        load_config(path.to_str().unwrap())
    }

    #[test]
    fn tailscale_config_loads_defaults_for_existing_server_files() {
        let config = load_server_fixture("").unwrap();
        let actual = toml::Value::try_from(config).unwrap();
        assert_eq!(actual.get("tailscale"), Some(&tailscale_defaults()));
    }

    #[test]
    fn tailscale_config_supports_partial_operator_overrides() {
        let config = load_server_fixture(
            "[tailscale]\nimage='registry.example/tailscale:custom'\nsocket_path='/run/custom/daemon.sock'\nexec_timeout_secs=90",
        ).unwrap();
        let actual = toml::Value::try_from(config).unwrap();
        let mut expected = tailscale_defaults();
        expected["image"] = "registry.example/tailscale:custom".into();
        expected["socket_path"] = "/run/custom/daemon.sock".into();
        expected["exec_timeout_secs"] = 90.into();
        assert_eq!(actual.get("tailscale"), Some(&expected));
    }

    #[test]
    fn tailscale_config_generation_writes_defaults_to_server_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sherpa.toml");
        create_config(&default_config(), path.to_str().unwrap()).unwrap();
        let actual: toml::Value = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(actual.get("tailscale"), Some(&tailscale_defaults()));
    }

    #[test]
    fn tailscale_config_rejects_malformed_and_unknown_settings() {
        for extra in [
            "[tailscale]\nexec_timeout_secs='slow'",
            "[tailscale]\nexec_timeout_sec=10",
            "[tailscale]\nstop_timeout_secs=-1",
        ] {
            assert!(load_server_fixture(extra).is_err(), "accepted {extra}");
        }
    }

    #[test]
    fn test_build_websocket_url_with_tls() {
        let mut config = default_config();
        config.tls.enabled = true;
        config.server_ipv4 = Ipv4Addr::new(10, 0, 0, 1);
        config.ws_port = 3030;
        config.server_connection.url = None;
        assert_eq!(build_websocket_url(&config), "wss://10.0.0.1:3030/ws");
    }

    #[test]
    fn test_build_websocket_url_without_tls() {
        let mut config = default_config();
        config.tls.enabled = false;
        config.server_ipv4 = Ipv4Addr::new(10, 0, 0, 1);
        config.ws_port = 3030;
        config.server_connection.url = None;
        assert_eq!(build_websocket_url(&config), "ws://10.0.0.1:3030/ws");
    }

    #[test]
    fn test_build_websocket_url_explicit_url_takes_precedence() {
        let mut config = default_config();
        config.server_connection.url = Some("wss://custom.host:9999/ws".to_string());
        assert_eq!(build_websocket_url(&config), "wss://custom.host:9999/ws");
    }

    #[test]
    fn test_build_client_websocket_url_with_tls() {
        let mut config = ClientConfig::default();
        config.tls.enabled = true;
        config.server_ipv4 = Ipv4Addr::new(10, 0, 0, 1);
        config.ws_port = 3030;
        assert_eq!(
            build_client_websocket_url(&config),
            "wss://10.0.0.1:3030/ws"
        );
    }

    #[test]
    fn test_build_client_websocket_url_without_tls() {
        let mut config = ClientConfig::default();
        config.tls.enabled = false;
        config.server_ipv4 = Ipv4Addr::new(10, 0, 0, 1);
        config.ws_port = 3030;
        assert_eq!(build_client_websocket_url(&config), "ws://10.0.0.1:3030/ws");
    }

    #[test]
    fn test_default_config_has_sensible_values() {
        let config = default_config();
        assert_eq!(config.server_ipv4, Ipv4Addr::new(127, 0, 0, 1));
        assert_eq!(config.ws_port, SHERPA_SERVER_WS_PORT);
        assert_eq!(config.http_port, SHERPA_SERVER_HTTP_PORT);
        assert!(!config.images_dir.is_empty());
        assert!(!config.containers_dir.is_empty());
        assert!(!config.container_images.is_empty());
    }

    #[test]
    fn test_sherpa_from_base_dir() {
        let sherpa = Sherpa::from_base_dir("/opt/sherpa".to_string());
        assert_eq!(sherpa.base_dir, "/opt/sherpa");
        assert!(sherpa.config_dir.starts_with("/opt/sherpa"));
        assert!(sherpa.config_file_path.starts_with("/opt/sherpa"));
        assert!(sherpa.ssh_dir.starts_with("/opt/sherpa"));
        assert!(sherpa.images_dir.starts_with("/opt/sherpa"));
        assert!(sherpa.containers_dir.starts_with("/opt/sherpa"));
        assert!(sherpa.bins_dir.starts_with("/opt/sherpa"));
    }
}
