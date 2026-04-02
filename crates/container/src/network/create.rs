use std::collections::HashMap;

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::{Ipam, IpamConfig, NetworkCreateRequest};
use tracing::instrument;

#[instrument(skip(docker), level = "debug")]
pub async fn create_docker_bridge_network(
    docker: &Docker,
    name: &str,
    ipv4_prefix: Option<String>,
    ipv6_prefix: Option<String>,
    bridge: &str,
) -> Result<()> {
    let mut ipam_configs = vec![IpamConfig {
        subnet: ipv4_prefix,
        ..Default::default()
    }];

    let enable_ipv6 = ipv6_prefix.is_some();
    if let Some(v6_prefix) = ipv6_prefix {
        ipam_configs.push(IpamConfig {
            subnet: Some(v6_prefix),
            ..Default::default()
        });
    }

    let ipam = Ipam {
        driver: Some("default".to_string()),
        config: Some(ipam_configs),
        ..Default::default()
    };

    let mut options = HashMap::new();
    options.insert(
        "com.docker.network.bridge.name".to_string(),
        bridge.to_string(),
    );
    let create_request = NetworkCreateRequest {
        name: name.to_owned(),
        driver: Some("bridge".to_owned()),
        options: Some(options),
        ipam: Some(ipam),
        internal: Some(false),
        enable_ipv6: Some(enable_ipv6),
        ..Default::default()
    };

    match docker.create_network(create_request).await {
        Ok(response) => {
            tracing::info!(network_name = %name, response = ?response, "Container network created")
        }
        Err(e) => {
            tracing::error!(network_name = %name, error = %e, "Error creating container network")
        }
    }

    Ok(())
}

/// Create a Docker macvlan network that uses an existing Linux bridge.
/// This allows Docker containers to attach to pre-created bridges used for VM-VM or VM-Container links.
///
/// # Arguments
/// * `docker` - Docker connection
/// * `parent_interface` - Name of the pre-existing Linux bridge (e.g., "bra0-12345")
/// * `network_name` - Name for the Docker network (e.g., "sherpa-link-a0-12345")
///
/// # Notes
/// * Uses macvlan driver which provides pure L2 connectivity without requiring IP addressing
/// * No IPAM configuration - containers get no IPs on this network
/// * Relies on the bridge already being created via rtnetlink
#[instrument(skip(docker), level = "debug")]
pub async fn create_docker_macvlan_network(
    docker: &Docker,
    parent_interface: &str,
    network_name: &str,
) -> Result<()> {
    let mut options = HashMap::new();
    options.insert("parent".to_string(), parent_interface.to_string());
    options.insert("macvlan_mode".to_string(), "passthru".to_string());

    let create_request = NetworkCreateRequest {
        name: network_name.to_owned(),
        driver: Some("macvlan".to_owned()),
        options: Some(options),
        internal: Some(false),
        ipam: None,
        enable_ipv4: Some(false),
        enable_ipv6: Some(false),
        ..Default::default()
    };

    docker
        .create_network(create_request)
        .await
        .with_context(|| {
            format!(
                "Failed to create Docker macvlan network '{}' on bridge '{}'",
                network_name, parent_interface
            )
        })?;

    tracing::info!(
        network_name = %network_name,
        parent_interface = %parent_interface,
        "Created Docker macvlan network on bridge (passthru mode)"
    );

    Ok(())
}

/// Create a Docker macvlan network in **bridge** mode on an existing Linux bridge.
/// Unlike `create_docker_macvlan_network` (passthru), bridge mode allows multiple
/// macvlan networks to share the same parent bridge — required for isolated bridges
/// that carry multiple disabled interfaces.
#[instrument(skip(docker), level = "debug")]
pub async fn create_docker_macvlan_bridge_network(
    docker: &Docker,
    parent_interface: &str,
    network_name: &str,
) -> Result<()> {
    let mut options = HashMap::new();
    options.insert("parent".to_string(), parent_interface.to_string());
    options.insert("macvlan_mode".to_string(), "bridge".to_string());

    let create_request = NetworkCreateRequest {
        name: network_name.to_owned(),
        driver: Some("macvlan".to_owned()),
        options: Some(options),
        internal: Some(false),
        ipam: None,
        enable_ipv4: Some(false),
        enable_ipv6: Some(false),
        ..Default::default()
    };

    docker
        .create_network(create_request)
        .await
        .with_context(|| {
            format!(
                "Failed to create Docker macvlan bridge-mode network '{}' on bridge '{}'",
                network_name, parent_interface
            )
        })?;

    tracing::info!(
        network_name = %network_name,
        parent_interface = %parent_interface,
        "Created Docker macvlan network on bridge (bridge mode)"
    );

    Ok(())
}
