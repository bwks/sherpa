use std::collections::HashMap;
use std::fmt;

use anyhow::{Context, Result, bail};
use bollard::Docker;
use bollard::models::{Ipam, IpamConfig, Network, NetworkCreateRequest};
use bollard::query_parameters::InspectNetworkOptions;
use tracing::instrument;

use crate::error::ContainerError;
use crate::status::{is_conflict, is_missing};

/// The Docker network option that names the Linux bridge of a bridge network.
const BRIDGE_NAME_OPTION: &str = "com.docker.network.bridge.name";

/// A subnet of a Docker bridge network, with the gateway Docker gives the bridge in it.
#[derive(Debug, Clone)]
pub struct BridgeSubnet {
    pub subnet: String,
    pub gateway: Option<String>,
}

/// A Docker network of a given name, as found against the subnets and bridge asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExistingBridgeNetwork {
    Absent,
    Matches,
    Differs(Vec<NetworkDifference>),
}

/// One way a Docker network differs from the subnets, gateways and bridge asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkDifference {
    MissingSubnet {
        subnet: String,
    },
    Gateway {
        subnet: String,
        expected: String,
        found: Option<String>,
    },
    UnallocatedSubnet {
        found: Option<String>,
    },
    Bridge {
        expected: String,
        found: Option<String>,
    },
}

/// Look for the Docker network `name` and compare it with the subnets, gateways and bridge asked
/// for. A network that differs comes back with each difference.
///
/// # Errors
///
/// Fails when Docker cannot be asked about the network.
#[instrument(skip(docker), level = "debug")]
pub async fn inspect_existing_bridge_network(
    docker: &Docker,
    name: &str,
    subnets: &[BridgeSubnet],
    bridge: &str,
) -> Result<ExistingBridgeNetwork> {
    let existing = match docker
        .inspect_network(name, None::<InspectNetworkOptions>)
        .await
    {
        Ok(existing) => existing,
        Err(error) if is_missing(&error) => return Ok(ExistingBridgeNetwork::Absent),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to inspect existing Docker network '{name}'"));
        }
    };
    let differences = network_differences(&existing, subnets, bridge);
    if differences.is_empty() {
        return Ok(ExistingBridgeNetwork::Matches);
    }
    Ok(ExistingBridgeNetwork::Differs(differences))
}

/// Create a Docker bridge network on the Linux bridge `bridge`, with the given subnets. A
/// network named `name` that already exists, such as on a retried `up`, is reused only when it
/// has these subnets, gateways and bridge; one that differs is refused and left in place.
///
/// # Errors
///
/// Fails when Docker refuses to create the network, for example for an invalid subnet, or a
/// network of that name exists with other subnets, gateways or another bridge.
#[instrument(skip(docker), level = "debug")]
pub async fn create_docker_bridge_network(
    docker: &Docker,
    name: &str,
    ipv4: BridgeSubnet,
    ipv6: Option<BridgeSubnet>,
    bridge: &str,
) -> Result<()> {
    let enable_ipv6 = ipv6.is_some();
    let subnets: Vec<BridgeSubnet> = [Some(ipv4), ipv6].into_iter().flatten().collect();
    let ipam_configs = subnets
        .iter()
        .map(|wanted| IpamConfig {
            subnet: Some(wanted.subnet.clone()),
            gateway: wanted.gateway.clone(),
            ..Default::default()
        })
        .collect();

    let ipam = Ipam {
        driver: Some("default".to_string()),
        config: Some(ipam_configs),
        ..Default::default()
    };

    let options = HashMap::from([(BRIDGE_NAME_OPTION.to_string(), bridge.to_string())]);
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
            match inspect_existing_bridge_network(docker, name, &subnets, bridge).await? {
                ExistingBridgeNetwork::Matches => {
                    tracing::debug!(network_name = %name, "Container network already exists and is reused");
                    Ok(())
                }
                ExistingBridgeNetwork::Differs(differences) => {
                    Err(ContainerError::NetworkDiffers {
                        name: name.to_owned(),
                        differences,
                    }
                    .into())
                }
                ExistingBridgeNetwork::Absent => {
                    bail!("Docker network '{name}' was removed while it was being created")
                }
            }
        }
        Err(error) => Err(error).with_context(|| {
            format!("Failed to create Docker bridge network '{name}' on bridge '{bridge}'")
        }),
    }
}

/// How the Docker network `existing` differs from the subnets and bridge a lab was allocated,
/// one entry per difference.
fn network_differences(
    existing: &Network,
    subnets: &[BridgeSubnet],
    bridge: &str,
) -> Vec<NetworkDifference> {
    let configs = existing
        .ipam
        .as_ref()
        .and_then(|ipam| ipam.config.as_deref())
        .unwrap_or_default();
    let mut differences = Vec::new();
    for wanted in subnets {
        let found = configs
            .iter()
            .find(|config| config.subnet.as_deref() == Some(wanted.subnet.as_str()));
        match (found, &wanted.gateway) {
            (None, _) => differences.push(NetworkDifference::MissingSubnet {
                subnet: wanted.subnet.clone(),
            }),
            (Some(config), Some(gateway)) if config.gateway.as_ref() != Some(gateway) => {
                differences.push(NetworkDifference::Gateway {
                    subnet: wanted.subnet.clone(),
                    expected: gateway.clone(),
                    found: config.gateway.clone(),
                });
            }
            (Some(_), _) => {}
        }
    }
    for config in configs {
        if !subnets
            .iter()
            .any(|wanted| config.subnet.as_ref() == Some(&wanted.subnet))
        {
            differences.push(NetworkDifference::UnallocatedSubnet {
                found: config.subnet.clone(),
            });
        }
    }
    let existing_bridge = existing
        .options
        .as_ref()
        .and_then(|options| options.get(BRIDGE_NAME_OPTION));
    if existing_bridge.map(String::as_str) != Some(bridge) {
        differences.push(NetworkDifference::Bridge {
            expected: bridge.to_owned(),
            found: existing_bridge.cloned(),
        });
    }
    differences
}

impl fmt::Display for NetworkDifference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSubnet { subnet } => write!(f, "it has no subnet {subnet}"),
            Self::Gateway {
                subnet,
                expected,
                found,
            } => write!(
                f,
                "its gateway in {subnet} is {}, not {expected}",
                found.as_deref().unwrap_or("unset")
            ),
            Self::UnallocatedSubnet { found } => write!(
                f,
                "it has subnet {}, which the lab was not allocated",
                found.as_deref().unwrap_or("unset")
            ),
            Self::Bridge { expected, found } => write!(
                f,
                "its bridge is {}, not {expected}",
                found.as_deref().unwrap_or("unset")
            ),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_docker::{Reply, mock_docker};

    #[tokio::test]
    async fn management_network_008_a_docker_management_network_already_there_with_the_labs_subnets_and_bridge_is_reused()
     {
        let (docker, mock) = mock_docker(vec![
            Reply::Json(
                "/networks/create",
                "409 Conflict",
                r#"{"message":"network with name sherpa-mgmt-lab1 already exists"}"#,
            ),
            Reply::Json(
                "/networks/sherpa-mgmt-lab1",
                "200 OK",
                r#"{"Name":"sherpa-mgmt-lab1","IPAM":{"Config":[{"Subnet":"172.31.2.0/24","Gateway":"172.31.2.1"},{"Subnet":"fd00:b00b:0:9::/64","Gateway":"fd00:b00b:0:9::1"}]},"Options":{"com.docker.network.bridge.name":"brm-lab1"}}"#,
            ),
        ]);
        create_docker_bridge_network(
            &docker,
            "sherpa-mgmt-lab1",
            BridgeSubnet {
                subnet: "172.31.2.0/24".to_owned(),
                gateway: Some("172.31.2.1".to_owned()),
            },
            Some(BridgeSubnet {
                subnet: "fd00:b00b:0:9::/64".to_owned(),
                gateway: Some("fd00:b00b:0:9::1".to_owned()),
            }),
            "brm-lab1",
        )
        .await
        .unwrap();
        mock.finish();
    }

    #[tokio::test]
    async fn management_network_011_a_network_that_differs_when_docker_finds_it_at_creation_fails_the_creation_with_each_difference()
     {
        let (docker, mock) = mock_docker(vec![
            Reply::Json(
                "/networks/create",
                "409 Conflict",
                r#"{"message":"network with name sherpa-mgmt-lab1 already exists"}"#,
            ),
            Reply::Json(
                "/networks/sherpa-mgmt-lab1",
                "200 OK",
                r#"{"Name":"sherpa-mgmt-lab1","IPAM":{"Config":[{"Subnet":"172.31.1.0/24","Gateway":"172.31.1.1"},{"Subnet":"fd00:b00b:0:9::/64","Gateway":"fd00:b00b:0:9::7"}]},"Options":{"com.docker.network.bridge.name":"brm-old"}}"#,
            ),
        ]);
        let error = create_docker_bridge_network(
            &docker,
            "sherpa-mgmt-lab1",
            BridgeSubnet {
                subnet: "172.31.2.0/24".to_owned(),
                gateway: Some("172.31.2.1".to_owned()),
            },
            Some(BridgeSubnet {
                subnet: "fd00:b00b:0:9::/64".to_owned(),
                gateway: Some("fd00:b00b:0:9::1".to_owned()),
            }),
            "brm-lab1",
        )
        .await
        .unwrap_err();
        mock.finish();

        let Some(ContainerError::NetworkDiffers { name, differences }) =
            error.downcast_ref::<ContainerError>()
        else {
            panic!("not a network that differs: {error:#}");
        };
        assert_eq!(name, "sherpa-mgmt-lab1");
        assert_eq!(
            differences,
            &[
                NetworkDifference::MissingSubnet {
                    subnet: "172.31.2.0/24".to_owned(),
                },
                NetworkDifference::Gateway {
                    subnet: "fd00:b00b:0:9::/64".to_owned(),
                    expected: "fd00:b00b:0:9::1".to_owned(),
                    found: Some("fd00:b00b:0:9::7".to_owned()),
                },
                NetworkDifference::UnallocatedSubnet {
                    found: Some("172.31.1.0/24".to_owned()),
                },
                NetworkDifference::Bridge {
                    expected: "brm-lab1".to_owned(),
                    found: Some("brm-old".to_owned()),
                },
            ]
        );
    }
}
