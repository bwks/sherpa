//! The lab's subnets and networks: reserving the management, loopback and IPv6 subnets with
//! the lab record, the management networks (phase 4), container
//! data networks (phase 6), shared bridges (phase 7) and container network attachments.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, anyhow, bail};
use container::{ContainerError, ExistingBridgeNetwork};
use db::LabDetails;
use tracing::instrument;

use super::context::LabBuild;
use super::links::P2pContainerVeth;
use crate::konst::{
    SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX, SHERPA_MANAGEMENT_NETWORK_IPV6,
    SHERPA_MANAGEMENT_NETWORK_NAME,
};
use crate::services::container_networks;
use crate::services::error::ServiceError;
use crate::services::p2p_links::{ContainerVeth, bridge_member_host_veth};

use shared::data;
use shared::data::{LabSubnets, RecordId, StatusKind};
use shared::konst::{MANAGEMENT_GATEWAY_HOST_OFFSET, SHERPA_MANAGEMENT_IP_INDEX};
use shared::util;

const SHERPA_LOOPBACK_PREFIX_IPV6: &str = "fd00:1001::/48";
/// Attempts to reserve lab subnets when concurrent labs take the chosen ones first.
const LAB_SUBNET_ALLOCATION_ATTEMPTS: u32 = 8;

/// A lab `up` has reserved: its record, `Creating`, and the subnets it holds. Only the `up` that
/// reserved the lab has one, so only it builds the lab, releases it, or rolls it back.
pub(super) struct ReservedLab {
    pub(super) record: data::DbLab,
    pub(super) subnets: LabSubnets,
}

/// The lab's management network, IPv4 and IPv6, from its reserved subnets.
pub(super) fn management_network(subnets: &LabSubnets) -> Result<data::SherpaNetwork> {
    let v4 = subnets.management_subnet;
    let v6 = subnets.ipv6_management_subnet;
    Ok(data::SherpaNetwork {
        v4: data::NetworkV4 {
            prefix: v4,
            first: subnets.gateway_ipv4,
            last: v4.broadcast(),
            boot_server: subnets.router_ipv4,
            network: v4.network(),
            subnet_mask: v4.netmask(),
            hostmask: v4.hostmask(),
            prefix_length: v4.prefix_len(),
        },
        v6: Some(data::NetworkV6 {
            prefix: v6,
            first: subnets.gateway_ipv6,
            last: util::get_ipv6_addr(&v6, u32::MAX)?,
            boot_server: subnets.router_ipv6,
            network: v6.network(),
            prefix_length: v6.prefix_len(),
        }),
    })
}

/// The name of the lab's management network, in libvirt and in Docker.
fn management_network_name(lab_id: &str) -> String {
    format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}")
}

/// The Linux bridge of the lab's management network.
fn management_bridge_name(lab_id: &str) -> String {
    format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}")
}

/// The subnets and gateways of the lab's Docker management network: IPv4, then IPv6.
fn docker_management_subnets(subnets: &LabSubnets) -> [container::BridgeSubnet; 2] {
    [
        container::BridgeSubnet {
            subnet: subnets.management_subnet.to_string(),
            gateway: Some(subnets.gateway_ipv4.to_string()),
        },
        container::BridgeSubnet {
            subnet: subnets.ipv6_management_subnet.to_string(),
            gateway: Some(subnets.gateway_ipv6.to_string()),
        },
    ]
}

/// Refuse an `up` whose lab's Docker management network is already there with other subnets,
/// gateways or another bridge, before anything is created, so no rollback can remove it. One that
/// matches is left to be reused.
#[instrument(skip_all, fields(%lab_id))]
pub(super) async fn check_existing_docker_management_network(
    docker: &container::Docker,
    lab_id: &str,
    subnets: &LabSubnets,
) -> Result<()> {
    let name = management_network_name(lab_id);
    match container::inspect_existing_bridge_network(
        docker,
        &name,
        &docker_management_subnets(subnets),
        &management_bridge_name(lab_id),
    )
    .await?
    {
        ExistingBridgeNetwork::Differs(differences) => Err(ServiceError::Conflict(
            ContainerError::NetworkDiffers { name, differences }.to_string(),
        )
        .into()),
        ExistingBridgeNetwork::Matches => {
            tracing::info!(network = %name, "Reusing the Docker management network already there");
            Ok(())
        }
        ExistingBridgeNetwork::Absent => Ok(()),
    }
}

/// Create the lab's management network in libvirt (NAT, with DHCP off) and in Docker, on the
/// same bridge, so VMs and containers share it.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_management_networks(
    build: &LabBuild<'_>,
    subnets: &LabSubnets,
) -> Result<()> {
    let lab_id = build.lab_id;
    let network_name = management_network_name(lab_id);
    let bridge_name = management_bridge_name(lab_id);
    let _ = build.progress.send_status(
        format!("Creating management network: {network_name}"),
        StatusKind::Progress,
    );
    libvirt::NatNetwork {
        network_name: network_name.clone(),
        bridge_name: bridge_name.clone(),
        ipv4_address: subnets.gateway_ipv4,
        ipv4_netmask: subnets.management_subnet.netmask(),
        ipv6_address: Some(subnets.gateway_ipv6),
        ipv6_prefix_length: Some(subnets.ipv6_management_subnet.prefix_len()),
    }
    .create(build.qemu_conn)?;
    tracing::info!(network = %network_name, bridge = %bridge_name, "Created libvirt NAT network");

    let [ipv4, ipv6] = docker_management_subnets(subnets);
    container::create_docker_bridge_network(
        build.docker_conn,
        &network_name,
        ipv4,
        Some(ipv6),
        &bridge_name,
    )
    .await?;
    tracing::info!(network = %network_name, "Created Docker bridge network");
    Ok(())
}

/// The lab record fields for a set of reserved subnets.
pub(super) fn lab_networks(subnets: &LabSubnets) -> db::LabNetworks {
    db::LabNetworks {
        management_network: subnets.management_subnet.to_string(),
        gateway_ipv4: subnets.gateway_ipv4.to_string(),
        router_ipv4: subnets.router_ipv4.to_string(),
        management_network_v6: Some(subnets.ipv6_management_subnet.to_string()),
        loopback_network_v6: Some(subnets.ipv6_loopback_subnet.to_string()),
        gateway_ipv6: Some(subnets.gateway_ipv6.to_string()),
        router_ipv6: Some(subnets.router_ipv6.to_string()),
    }
}

/// Choose free subnets: not held by any lab, and for management networks not
/// overlapping a network already on a host interface (another lab's bridge, a stale
/// leftover or an unrelated service).
#[instrument(skip(db, config), level = "debug")]
pub(super) async fn allocate_lab_networks(
    db: &db::Database,
    config: &data::Config,
) -> Result<LabSubnets> {
    let host_networks = util::get_interface_networks()
        .context("Failed to read host IPv4 networks for subnet allocation")?;
    let host_networks_v6 = util::get_interface_networks_v6()
        .context("Failed to read host IPv6 networks for subnet allocation")?;

    let used_management = db::get_used_management_networks(db)
        .await
        .context("Failed to query existing management networks")?;
    let management_subnet = util::allocate_management_subnet(
        &config.management_prefix_ipv4,
        &used_management,
        &host_networks,
    )
    .context("Failed to allocate management subnet for lab")?;

    let ipv6_management_prefix = match config.management_prefix_ipv6 {
        Some(prefix) => prefix,
        None => util::get_ipv6_network(SHERPA_MANAGEMENT_NETWORK_IPV6)
            .context("Failed to parse default IPv6 management prefix")?,
    };
    let ipv6_loopback_prefix = util::get_ipv6_network(SHERPA_LOOPBACK_PREFIX_IPV6)
        .context("Failed to parse IPv6 loopback prefix")?;
    let used_ipv6_management = db::get_used_ipv6_management_networks(db)
        .await
        .context("Failed to query existing IPv6 management networks")?;
    let ipv6_management_subnet = util::allocate_ipv6_management_subnet(
        &ipv6_management_prefix,
        &used_ipv6_management,
        &host_networks_v6,
    )
    .context("Failed to allocate IPv6 management subnet for lab")?;
    let used_ipv6_loopback = db::get_used_ipv6_loopback_networks(db)
        .await
        .context("Failed to query existing IPv6 loopback networks")?;
    let ipv6_loopback_subnet =
        util::allocate_ipv6_loopback_subnet(&ipv6_loopback_prefix, &used_ipv6_loopback)
            .context("Failed to allocate IPv6 loopback subnet for lab")?;

    Ok(LabSubnets {
        management_subnet,
        gateway_ipv4: util::get_ipv4_addr(&management_subnet, MANAGEMENT_GATEWAY_HOST_OFFSET)?,
        router_ipv4: util::get_ipv4_addr(&management_subnet, SHERPA_MANAGEMENT_IP_INDEX)?,
        ipv6_management_subnet,
        ipv6_loopback_subnet,
        gateway_ipv6: util::get_ipv6_addr(&ipv6_management_subnet, MANAGEMENT_GATEWAY_HOST_OFFSET)?,
        router_ipv6: util::get_ipv6_addr(&ipv6_management_subnet, SHERPA_MANAGEMENT_IP_INDEX)?,
    })
}

/// Whether any chosen subnet is now held by a lab record.
#[instrument(skip(db), level = "debug")]
pub(super) async fn networks_taken(db: &db::Database, networks: &LabSubnets) -> Result<bool> {
    Ok(db::get_used_management_networks(db)
        .await?
        .contains(&networks.management_subnet)
        || db::get_used_ipv6_management_networks(db)
            .await?
            .contains(&networks.ipv6_management_subnet)
        || db::get_used_ipv6_loopback_networks(db)
            .await?
            .contains(&networks.ipv6_loopback_subnet))
}

/// Reserve the lab: allocate its subnets and insert its record, `Creating`, that holds them.
///
/// The record is `up`'s reservation of the lab ID and the subnets: a second `up` of the same
/// lab fails here, before it creates anything, so it has nothing to roll back. When a
/// concurrent `up` of another lab takes a chosen subnet first, the insert fails, and allocation
/// is repeated with fresh state. Any other failure is returned as is.
///
/// # Errors
/// A `ServiceError::Conflict` when a lab already has the lab ID; any database failure.
#[instrument(skip(db, config, user), fields(%lab_id), level = "debug")]
pub(super) async fn reserve_lab(
    db: &db::Database,
    config: &data::Config,
    lab_name: &str,
    lab_id: &str,
    user: &data::DbUser,
    details: &LabDetails,
) -> Result<ReservedLab> {
    for attempt in 1..=LAB_SUBNET_ALLOCATION_ATTEMPTS {
        let networks = allocate_lab_networks(db, config).await?;
        match db::create_lab(
            db,
            lab_name,
            lab_id,
            user,
            &lab_networks(&networks),
            details,
        )
        .await
        {
            Ok(lab) => {
                tracing::info!(
                    lab_id = %lab_id,
                    management_subnet = %networks.management_subnet,
                    ipv6_management_subnet = %networks.ipv6_management_subnet,
                    ipv6_loopback_subnet = %networks.ipv6_loopback_subnet,
                    "Reserved the lab and its subnets"
                );
                return Ok(ReservedLab {
                    record: lab,
                    subnets: networks,
                });
            }
            Err(error) => {
                if let Some(existing) = db::find_lab(db, lab_id).await? {
                    return Err(lab_id_taken(lab_id, &existing, lab_name, user.id.as_ref()));
                }
                if !networks_taken(db, &networks).await? {
                    return Err(error).context("Failed to create lab record in database");
                }
                tracing::warn!(
                    lab_id = %lab_id,
                    attempt,
                    "A concurrent lab took an allocated subnet; allocating again"
                );
            }
        }
    }
    bail!(
        "Could not reserve free subnets for lab {lab_id} after {LAB_SUBNET_ALLOCATION_ATTEMPTS} attempts"
    )
}

/// The error for an `up` whose lab ID is in use. Either this lab exists already, or another
/// lab has the same ID: IDs are a hash of the owner and the lab name, so two owners' labs can
/// collide. The other lab belongs to someone else, so it is not named.
pub(super) fn lab_id_taken(
    lab_id: &str,
    existing: &data::DbLab,
    lab_name: &str,
    owner: Option<&RecordId>,
) -> anyhow::Error {
    if existing.name == lab_name && owner == Some(&existing.user) {
        ServiceError::LabExists {
            lab_name: lab_name.to_owned(),
            lab_id: lab_id.to_owned(),
            phase: existing.phase,
        }
        .into()
    } else {
        ServiceError::LabIdCollision {
            lab_name: lab_name.to_owned(),
            lab_id: lab_id.to_owned(),
        }
        .into()
    }
}

/// Create the Docker networks of the disabled interfaces of containers on no link or bridge.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_isolated_container_networks(
    build: &LabBuild<'_>,
    node_setup_data: &[data::NodeSetupData],
    lab_node_data: &[data::LabNodeData],
    veth_container_names: &HashSet<String>,
) -> Result<()> {
    let LabBuild {
        lab_id,
        docker_conn,
        ..
    } = *build;

    tracing::info!(lab_id = %lab_id, "Creating Docker networks for disabled container interfaces");

    let mut docker_net_count = 0;
    // Create Docker macvlan bridge-mode networks for disabled interfaces on container nodes
    for nsd in node_setup_data {
        let Some(iso_network) = &nsd.isolated_network else {
            continue;
        };

        let node_data = match lab_node_data.iter().find(|n| n.name == nsd.name) {
            Some(n) if n.model_kind.kind() == data::NodeKind::Container => n,
            _ => continue,
        };
        if veth_container_names.contains(&nsd.name) {
            continue;
        }

        for iface in &nsd.interfaces {
            if !matches!(iface.data, data::NodeInterface::Disabled) {
                continue;
            }
            let docker_net_name =
                container_networks::isolated_network_name(&node_data.name, iface.index, lab_id);
            tracing::info!(
                lab_id = %lab_id,
                node = %node_data.name,
                interface = %iface.name,
                network = %docker_net_name,
                bridge = %iso_network.bridge_name,
                "Creating isolated Docker macvlan bridge-mode network"
            );
            container::create_docker_macvlan_bridge_network(
                docker_conn,
                &iso_network.bridge_name,
                &docker_net_name,
            )
            .await?;
            docker_net_count += 1;
        }
    }

    tracing::info!(
        lab_id = %lab_id,
        networks_created = docker_net_count,
        "Docker macvlan networks created"
    );

    Ok(())
}

/// Create each `[[bridges]]` private bridge and record it.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_shared_bridges(
    build: &LabBuild<'_>,
    lab_record_id: &RecordId,
    bridges_detailed: &[topology::BridgeDetailed],
    lab_node_data: &[data::LabNodeData],
    p2p_container_veths: &mut Vec<P2pContainerVeth>,
) -> Result<()> {
    let LabBuild {
        lab_id,
        db,
        progress,
        ..
    } = *build;

    tracing::info!(
        lab_id = %lab_id,
        bridge_count = bridges_detailed.len(),
        "Creating shared bridges"
    );

    for bridge in bridges_detailed {
        let _ = progress.send_status(
            format!(
                "Creating shared bridge #{} - {} ({} connections)",
                bridge.index,
                bridge.manifest_name,
                bridge.links.len()
            ),
            StatusKind::Progress,
        );

        tracing::info!(
            lab_id = %lab_id,
            bridge_num = bridge.index,
            bridge_name = %bridge.manifest_name,
            connections = bridge.links.len(),
            "Creating shared bridge"
        );

        network::create_bridge(&bridge.bridge_name, &bridge.libvirt_name).await?;

        tracing::debug!(
            lab_id = %lab_id,
            bridge = %bridge.bridge_name,
            libvirt_name = %bridge.libvirt_name,
            "Created bridge infrastructure"
        );

        let bridge_members =
            join_bridge_members(lab_id, bridge, lab_node_data, p2p_container_veths)?;

        db::create_bridge(
            db,
            bridge.index,
            bridge.bridge_name.clone(),
            bridge.libvirt_name.clone(),
            lab_record_id.clone(),
            bridge_members,
        )
        .await?;
    }

    tracing::info!(
        lab_id = %lab_id,
        bridges_created = bridges_detailed.len(),
        "All shared bridges created"
    );

    Ok(())
}

/// The bridge's members, for its record; container members' veths go to `p2p_container_veths`.
fn join_bridge_members(
    lab_id: &str,
    bridge: &topology::BridgeDetailed,
    lab_node_data: &[data::LabNodeData],
    p2p_container_veths: &mut Vec<P2pContainerVeth>,
) -> Result<Vec<data::BridgeMember>> {
    let mut bridge_members = vec![];
    for link in &bridge.links {
        let node_data = lab_node_data
            .iter()
            .find(|n| n.name == link.node_name)
            .ok_or_else(|| {
                anyhow!(
                    "Node {} in bridge {} not found in lab_node_data",
                    link.node_name,
                    bridge.manifest_name
                )
            })?;
        bridge_members.push(data::BridgeMember {
            node: db::get_node_id(&node_data.record)?,
            interface: link.interface_name.clone(),
        });

        if node_data.model_kind.kind() != data::NodeKind::Container {
            continue;
        }
        p2p_container_veths.push(P2pContainerVeth {
            node_name: link.node_name.clone(),
            veth: ContainerVeth {
                host_veth: bridge_member_host_veth(lab_id, node_data.index, link.interface_index),
                host_alias: format!(
                    "{}-bridge-host-{}::{}",
                    lab_id, link.node_name, link.interface_name
                ),
                interface_idx: link.interface_index,
                model_kind: node_data.model_kind,
                admin_down: false,
                bridge: Some(bridge.bridge_name.clone()),
            },
        });
    }
    Ok(bridge_members)
}

/// Set every node's isolated bridge down, so its disabled interfaces have no carrier and show
/// as not connected on the node (phase 11c). Containers join their bridge later, without
/// running anything in the image.
#[instrument(skip_all, level = "debug")]
pub(super) async fn set_isolated_bridges_down(
    node_setup_data: &[data::NodeSetupData],
) -> Result<()> {
    for node in node_setup_data {
        if let Some(network) = &node.isolated_network {
            tracing::info!(
                node_name = %node.name,
                bridge_name = %network.bridge_name,
                "Setting isolated bridge down to remove carrier from disabled interfaces"
            );
            network::set_link_state(&network.bridge_name, false).await?;
        }
    }
    Ok(())
}

/// Each container's Docker data network attachments, in interface order, admin down.
pub(super) fn container_data_attachments(
    lab_id: &str,
    node_setup_data: &[data::NodeSetupData],
    lab_node_data: &[data::LabNodeData],
    veth_container_names: &HashSet<String>,
) -> Result<HashMap<String, Vec<data::ContainerNetworkAttachment>>> {
    let mut container_link_networks: HashMap<String, Vec<data::ContainerNetworkAttachment>> =
        HashMap::new();

    for nsd in node_setup_data {
        let node_data = match lab_node_data.iter().find(|n| n.name == nsd.name) {
            Some(n) if n.model_kind.kind() == data::NodeKind::Container => n,
            _ => continue,
        };

        if veth_container_names.contains(&nsd.name) {
            container_link_networks.insert(nsd.name.clone(), vec![]);
            continue;
        }

        let attachments = container_networks::data_attachments(
            &nsd.name,
            node_data.model_kind,
            &nsd.interfaces,
            lab_id,
        )
        .with_context(|| {
            format!(
                "Failed to build network attachments for container {}",
                nsd.name
            )
        })?;

        container_link_networks.insert(nsd.name.clone(), attachments);
    }

    tracing::info!(
        lab_id = %lab_id,
        container_count = container_link_networks.len(),
        "Built container network mappings"
    );

    Ok(container_link_networks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::LabPhase;

    #[test]
    fn lab_addressing_009_the_management_network_spans_the_labs_subnets_from_the_gateway() {
        let subnets = LabSubnets {
            management_subnet: "172.31.4.0/24".parse().unwrap(),
            gateway_ipv4: "172.31.4.1".parse().unwrap(),
            router_ipv4: "172.31.4.2".parse().unwrap(),
            ipv6_management_subnet: "fd00:b00b:0:4::/64".parse().unwrap(),
            ipv6_loopback_subnet: "fd00:b00c:0:4::/64".parse().unwrap(),
            gateway_ipv6: "fd00:b00b:0:4::1".parse().unwrap(),
            router_ipv6: "fd00:b00b:0:4::2".parse().unwrap(),
        };
        let network = management_network(&subnets).unwrap();
        assert_eq!(network.v4.first.to_string(), "172.31.4.1");
        assert_eq!(network.v4.boot_server.to_string(), "172.31.4.2");
        assert_eq!(network.v4.last.to_string(), "172.31.4.255");
        assert_eq!(network.v4.prefix_length, 24);
        let v6 = network.v6.unwrap();
        assert_eq!(v6.first.to_string(), "fd00:b00b:0:4::1");
        assert_eq!(v6.prefix_length, 64);
    }

    async fn database_with(username: &str) -> (db::Database, data::DbUser) {
        let db = db::open_in_memory().await.unwrap();
        db::apply_schema(&db).await.unwrap();
        let user = db::create_user(&db, username.to_string(), "TestPass123!", false, vec![])
            .await
            .unwrap();
        (db, user)
    }

    fn config() -> data::Config {
        toml::from_str(
            r#"
            name = "test"
            vm_provider = "libvirt"
            qemu_bin = "/usr/bin/qemu-system-x86_64"
            images_dir = "/opt/sherpa/images"
            containers_dir = "/opt/sherpa/containers"
            bins_dir = "/opt/sherpa/bins"
            "#,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn lab_concurrency_004_a_second_reservation_of_a_lab_being_created_is_refused() {
        // The record is the reservation, taken before anything is created, so a second `up`
        // cannot fail later and have its rollback remove the first `up`'s lab.
        let (db, alice) = database_with("alice").await;
        let details = LabDetails::default();
        let first = reserve_lab(&db, &config(), "core", "ab12cd34", &alice, &details)
            .await
            .unwrap();
        assert_eq!(first.record.phase, LabPhase::Creating);

        let error = reserve_lab(&db, &config(), "core", "ab12cd34", &alice, &details)
            .await
            .err()
            .unwrap();

        assert_eq!(
            ServiceError::find(&error),
            Some(&ServiceError::LabExists {
                lab_name: "core".to_owned(),
                lab_id: "ab12cd34".to_owned(),
                phase: LabPhase::Creating,
            })
        );
        let lab = db::find_lab(&db, "ab12cd34").await.unwrap().unwrap();
        assert_eq!(lab.id, first.record.id);
        assert_eq!(lab.phase, LabPhase::Creating);
    }

    fn lab(name: &str, user: &RecordId, phase: LabPhase) -> data::DbLab {
        data::DbLab {
            id: None,
            lab_id: "ab12cd34".to_string(),
            name: name.to_string(),
            user: user.clone(),
            management_network: String::new(),
            gateway_ipv4: String::new(),
            router_ipv4: String::new(),
            tailscale_container_id: None,
            management_network_v6: None,
            gateway_ipv6: None,
            router_ipv6: None,
            loopback_network_v6: None,
            phase,
            client_host: None,
            client_folder: None,
            description: None,
        }
    }

    #[test]
    fn lab_phase_013_an_up_of_a_lab_that_is_not_ready_carries_its_phase() {
        let alice = RecordId::new("user", "alice".to_string());

        let stranded = lab_id_taken(
            "ab12cd34",
            &lab("core", &alice, LabPhase::DestroyFailed),
            "core",
            Some(&alice),
        );

        assert_eq!(
            stranded.downcast_ref::<ServiceError>(),
            Some(&ServiceError::LabExists {
                lab_name: "core".to_owned(),
                lab_id: "ab12cd34".to_owned(),
                phase: LabPhase::DestroyFailed,
            })
        );
    }

    #[test]
    fn a_lab_id_another_lab_uses_is_reported_without_naming_it() {
        let alice = RecordId::new("user", "alice".to_string());
        let bob = RecordId::new("user", "bob".to_string());

        let redeployed = lab_id_taken(
            "ab12cd34",
            &lab("core", &alice, LabPhase::Ready),
            "core",
            Some(&alice),
        );
        assert_eq!(
            redeployed.downcast_ref::<ServiceError>(),
            Some(&ServiceError::LabExists {
                lab_name: "core".to_owned(),
                lab_id: "ab12cd34".to_owned(),
                phase: LabPhase::Ready,
            })
        );

        let collision = lab_id_taken(
            "ab12cd34",
            &lab("x", &bob, LabPhase::Ready),
            "core",
            Some(&alice),
        );
        assert_eq!(
            collision.downcast_ref::<ServiceError>(),
            Some(&ServiceError::LabIdCollision {
                lab_name: "core".to_owned(),
                lab_id: "ab12cd34".to_owned(),
            })
        );
    }
}
