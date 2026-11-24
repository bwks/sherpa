use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::instrument;

use super::file_system::create_file;
use crate::data::{
    ClientConfig, Config, ContainerImage, HostMemorySettings, OtelConfig, ScannerConfig,
    ServerConnection, TailscaleGatewaySettings, TlsConfig, VmProviders,
};
use crate::konst::{
    QEMU_BIN, SHERPA_BINS_PATH, SHERPA_CONFIG_FILE, SHERPA_CONTAINERS_PATH, SHERPA_IMAGES_PATH,
    SHERPA_MANAGEMENT_NETWORK_IPV4, SHERPA_SERVER_HTTP_PORT, SHERPA_SERVER_WS_PORT,
};

/// Load the server config from `file_path`.
///
/// # Errors
///
/// Fails when the file cannot be read or is not a valid server config.
#[instrument(level = "debug")]
pub fn load_config(file_path: &str) -> Result<Config> {
    let expanded_path = shellexpand::tilde(file_path);
    let config_path = Path::new(expanded_path.as_ref());

    let contents = fs::read_to_string(config_path)
        .with_context(|| format!("Unable to read server config: {}", config_path.display()))?;
    toml::from_str(&contents).context("Invalid server configuration")
}

/// Build WebSocket URL from client config
#[must_use]
pub fn build_client_websocket_url(config: &ClientConfig) -> String {
    if let Some(ref url) = config.server_connection.url {
        return url.clone();
    }

    let scheme = if config.tls.enabled { "wss" } else { "ws" };
    let host = config.server_ipv4;
    let port = config.ws_port;

    format!("{scheme}://{host}:{port}/ws")
}

/// Write the client config to `path` as TOML.
///
/// # Errors
///
/// Fails when the config cannot be serialized or written.
pub fn create_client_config(config: &ClientConfig, path: &str) -> Result<()> {
    let toml_string = toml::to_string_pretty(&config)?;
    create_file(path, toml_string)?;
    Ok(())
}

/// Load the client config from `file_path`.
///
/// # Errors
///
/// Fails when the file cannot be read or is not a valid client config.
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

    let mgmt_prefix_ipv4 = SHERPA_MANAGEMENT_NETWORK_IPV4;

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
        allow_signup: false,
        container_images,
        management_prefix_ipv4: mgmt_prefix_ipv4,
        management_prefix_ipv6: None,
        server_connection: ServerConnection::default(),
        server_ipv4: Ipv4Addr::LOCALHOST,
        server_ipv6: None,
        ws_port: SHERPA_SERVER_WS_PORT,
        http_port: SHERPA_SERVER_HTTP_PORT,
        tls: TlsConfig::default(),
        otel: OtelConfig::default(),
        scanner: ScannerConfig::default(),
        tailscale: TailscaleGatewaySettings::default(),
        host_memory: HostMemorySettings::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_server_fixture(extra: &str) -> Result<Config> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("sherpa.toml");
        let mut config = toml::Value::try_from(default_config())?;
        config.as_table_mut().unwrap().remove("tailscale");
        fs::write(&path, format!("{}\n{extra}", toml::to_string(&config)?))?;
        load_config(path.to_str().unwrap())
    }

    #[test]
    fn tailnet_settings_002_a_wrong_type_unknown_setting_or_negative_timeout_is_refused_on_load() {
        for extra in [
            "[tailscale]\nexec_timeout_secs='slow'",
            "[tailscale]\nexec_timeout_sec=10",
            "[tailscale]\nstop_timeout_secs=-1",
        ] {
            assert!(load_server_fixture(extra).is_err(), "accepted {extra}");
        }
    }

    #[test]
    fn client_config_003_the_server_url_is_wss_with_tls_on_and_ws_with_it_off() {
        let mut config = ClientConfig {
            server_ipv4: Ipv4Addr::new(10, 0, 0, 1),
            ws_port: 3030,
            ..ClientConfig::default()
        };

        config.tls.enabled = true;
        assert_eq!(
            build_client_websocket_url(&config),
            "wss://10.0.0.1:3030/ws"
        );

        config.tls.enabled = false;
        assert_eq!(build_client_websocket_url(&config), "ws://10.0.0.1:3030/ws");
    }
}
