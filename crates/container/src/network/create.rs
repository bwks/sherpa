use std::collections::HashMap;

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::{Ipam, IpamConfig, NetworkCreateRequest};
use tracing::instrument;

use crate::status::is_conflict;

/// Create a Docker bridge network on the Linux bridge `bridge`, with the given subnets. A
/// network named `name` that already exists, such as on a retried `up`, counts as created.
///
/// # Errors
///
/// Fails when Docker refuses to create the network, for example for an invalid subnet.
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

    let options = HashMap::from([(
        "com.docker.network.bridge.name".to_string(),
        bridge.to_string(),
    )]);
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
            tracing::info!(network_name = %name, response = ?response, "Container network created");
            Ok(())
        }
        Err(error) if is_conflict(&error) => {
            tracing::debug!(network_name = %name, "Container network already exists");
            Ok(())
        }
        Err(error) => Err(error).with_context(|| {
            format!("Failed to create Docker bridge network '{name}' on bridge '{bridge}'")
        }),
    }
}

/// Create a Docker macvlan network in **bridge** mode on an existing Linux bridge.
/// Bridge mode allows multiple macvlan networks to share the same parent bridge, which isolated
/// bridges carrying several disabled interfaces need.
///
/// # Errors
///
/// Fails when Docker refuses to create the network.
#[instrument(skip(docker), level = "debug")]
pub async fn create_docker_macvlan_bridge_network(
    docker: &Docker,
    parent_interface: &str,
    network_name: &str,
) -> Result<()> {
    let options = HashMap::from([
        ("parent".to_string(), parent_interface.to_string()),
        ("macvlan_mode".to_string(), "bridge".to_string()),
    ]);

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
                "Failed to create Docker macvlan bridge-mode network '{network_name}' on bridge '{parent_interface}'"
            )
        })?;

    tracing::info!(
        network_name = %network_name,
        parent_interface = %parent_interface,
        "Created Docker macvlan network on bridge (bridge mode)"
    );

    Ok(())
}
