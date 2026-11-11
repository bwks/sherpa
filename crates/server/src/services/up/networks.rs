//! The lab's files, subnets and networks: reserving the management, loopback and IPv6
//! subnets with the lab record, the lab files and management networks (phase 4), container
//! data networks (phase 6), shared bridges (phase 7) and container network attachments.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use tracing::instrument;

use super::context::LabBuild;
use super::links::P2pContainerVeth;
use crate::services::container_networks;
use crate::services::error::ServiceError;
use crate::services::p2p_links::ContainerVeth;
use crate::tls::generator::write_file_with_permissions;

use shared::data;
use shared::data::{LabPhase, LabSubnets, RecordId, StatusKind};
use shared::konst::{
    BRIDGE_MEMBER_HOST_VETH_PREFIX, LAB_FILE_NAME, LAB_SUBNET_ALLOCATION_ATTEMPTS,
    MANAGEMENT_GATEWAY_HOST_OFFSET, SHERPA_LAB_MANIFEST_FILE, SHERPA_LOOPBACK_PREFIX,
    SHERPA_LOOPBACK_PREFIX_IPV6, SHERPA_MANAGEMENT_IP_INDEX,
    SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX, SHERPA_MANAGEMENT_NETWORK_IPV6,
    SHERPA_MANAGEMENT_NETWORK_NAME,
};
use shared::util;

/// A lab `up` has reserved: its record, `Creating`, and the subnets it holds. Only the `up` that
/// reserved the lab has one, so only it builds the lab, releases it, or rolls it back.
pub(super) struct ReservedLab {
    pub(super) record: data::DbLab,
    pub(super) subnets: LabSubnets,
}

/// Write the lab info file and, for later redeploys, a copy of the manifest. The manifest can
/// hold node secrets' paths and settings, so it is written private from the start.
pub(super) fn write_lab_files(
    lab_dir: &str,
    lab_info: &data::LabInfo,
    manifest: &topology::Manifest,
) -> Result<()> {
    util::create_dir(lab_dir)?;
    util::create_file(&format!("{lab_dir}/{LAB_FILE_NAME}"), lab_info.to_string())?;
    let manifest_toml =
        toml::to_string_pretty(manifest).context("Failed to serialize manifest for saving")?;
    write_file_with_permissions(
        Path::new(&format!("{lab_dir}/{SHERPA_LAB_MANIFEST_FILE}")),
        manifest_toml.as_bytes(),
        0o600,
    )
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

/// Create the lab's management network in libvirt (NAT, with DHCP off) and in Docker, on the
/// same bridge, so VMs and containers share it.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_management_networks(
    build: &LabBuild<'_>,
    subnets: &LabSubnets,
) -> Result<()> {
    let lab_id = build.lab_id;
    let network_name = format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}");
    let bridge_name = format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}");
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

    container::create_docker_bridge_network(
        build.docker_conn,
        &network_name,
        Some(subnets.management_subnet.to_string()),
        Some(subnets.ipv6_management_subnet.to_string()),
        &bridge_name,
    )
    .await?;
    tracing::info!(network = %network_name, "Created Docker bridge network");
    Ok(())
}

/// The lab record fields for a set of reserved subnets.
pub(super) fn lab_networks(subnets: &LabSubnets) -> db::LabNetworks {
    db::LabNetworks {
        loopback_network: subnets.loopback_subnet.to_string(),
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

    let loopback_prefix = util::get_ipv4_network(SHERPA_LOOPBACK_PREFIX)
        .context("Failed to parse loopback prefix")?;
    let used_loopback = db::get_used_loopback_networks(db)
        .await
        .context("Failed to query existing loopback networks")?;
    let loopback_subnet = util::allocate_loopback_subnet(&loopback_prefix, &used_loopback)
        .context("Failed to allocate loopback subnet for lab")?;

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
        loopback_subnet,
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
    Ok(db::get_used_loopback_networks(db)
        .await?
        .contains(&networks.loopback_subnet)
        || db::get_used_management_networks(db)
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
    details: &db::LabDetails,
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
                    loopback_subnet = %networks.loopback_subnet,
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
    let message = if existing.name == lab_name && owner == Some(&existing.user) {
        let state = match existing.phase {
            LabPhase::Ready => "already exists".to_string(),
            phase => format!("already exists and is {phase}"),
        };
        format!(
            "Lab {state}. Destroy the existing lab before starting it again.\n Lab name: {lab_name}\n Lab id: {lab_id}"
        )
    } else {
        format!(
            "Lab '{lab_name}' gets the lab ID {lab_id}, which another lab already uses. \
             Lab IDs come from the owner and the lab name, and these collide; rename the lab."
        )
    };
    ServiceError::Conflict(message).into()
}

/// Create a Docker macvlan network, on the node's isolated bridge, for each disabled interface of
/// a container not on a P2p link.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_isolated_container_networks(
    build: &LabBuild<'_>,
    node_setup_data: &[data::NodeSetupData],
    lab_node_data: &[data::LabNodeData],
    p2p_container_names: &HashSet<String>,
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

        // Only process container nodes (skip P2p containers — they use veth+netns)
        let node_data = match lab_node_data.iter().find(|n| n.name == nsd.name) {
            Some(n) if n.kind == data::NodeKind::Container => n,
            _ => continue,
        };
        if p2p_container_names.contains(&nsd.name) {
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

/// Create each `[[bridges]]` private bridge and record it. A container member on a P2p link
/// joins through a veth, added to `p2p_container_veths`; other container members get a Docker
/// macvlan network on the bridge, and VMs join through libvirt.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_shared_bridges(
    build: &LabBuild<'_>,
    lab_record_id: &RecordId,
    bridges_detailed: &[topology::BridgeDetailed],
    lab_node_data: &[data::LabNodeData],
    p2p_container_names: &HashSet<String>,
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

        let bridge_members = join_bridge_members(
            build,
            bridge,
            lab_node_data,
            p2p_container_names,
            p2p_container_veths,
        )
        .await?;

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

/// The bridge's members, for its record. A container member on a P2p link joins through a veth,
/// added to `p2p_container_veths`; another container member gets a Docker macvlan network on the
/// bridge now. VMs join through libvirt.
#[instrument(skip_all, fields(lab_id = %build.lab_id, bridge = %bridge.bridge_name), level = "debug")]
async fn join_bridge_members(
    build: &LabBuild<'_>,
    bridge: &topology::BridgeDetailed,
    lab_node_data: &[data::LabNodeData],
    p2p_container_names: &HashSet<String>,
    p2p_container_veths: &mut Vec<P2pContainerVeth>,
) -> Result<Vec<data::BridgeMember>> {
    let lab_id = build.lab_id;
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

        if node_data.kind != data::NodeKind::Container {
            continue;
        }

        if p2p_container_names.contains(&link.node_name) {
            // P2p containers take every data interface as a veth created into their
            // netns. The host end of a bridge member's veth joins the bridge.
            // Format: cb{node_idx}i{iface_idx}-{lab_id}
            p2p_container_veths.push(P2pContainerVeth {
                node_name: link.node_name.clone(),
                veth: ContainerVeth {
                    host_veth: format!(
                        "{BRIDGE_MEMBER_HOST_VETH_PREFIX}{}i{}-{lab_id}",
                        node_data.index, link.interface_index
                    ),
                    host_alias: format!(
                        "{}-bridge-host-{}::{}",
                        lab_id, link.node_name, link.interface_name
                    ),
                    interface_idx: link.interface_index,
                    node_model: node_data.model,
                    admin_down: false,
                    bridge: Some(bridge.bridge_name.clone()),
                },
            });
        } else {
            let docker_net_name = container_networks::bridge_member_network_name(
                &link.node_name,
                link.interface_index,
                lab_id,
            );
            tracing::info!(
                lab_id = %lab_id,
                node = %link.node_name,
                interface = %link.interface_name,
                network = %docker_net_name,
                bridge = %bridge.bridge_name,
                "Creating Docker macvlan bridge-mode network for bridge member"
            );
            container::create_docker_macvlan_bridge_network(
                build.docker_conn,
                &bridge.bridge_name,
                &docker_net_name,
            )
            .await?;
        }
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
            network::set_link_down(&network.bridge_name).await?;
        }
    }
    Ok(())
}

/// Each container's Docker data network attachments, in interface order: linked interfaces
/// active, disabled ones admin down. Containers on a P2p link get none; their data interfaces
/// are veths in their network namespace.
pub(super) fn container_data_attachments(
    lab_id: &str,
    node_setup_data: &[data::NodeSetupData],
    lab_node_data: &[data::LabNodeData],
    p2p_container_names: &HashSet<String>,
) -> Result<HashMap<String, Vec<data::ContainerNetworkAttachment>>> {
    // Build container network mappings from the full interface list.
    // This ensures all defined data interfaces appear in model-index order,
    // with linked interfaces active and disabled interfaces admin-down.
    let mut container_link_networks: HashMap<String, Vec<data::ContainerNetworkAttachment>> =
        HashMap::new();

    // Walk each container node's interfaces in index order.
    // For each data interface, produce a ContainerNetworkAttachment with
    // the correct docker network name and linux_interface_name.
    for nsd in node_setup_data {
        let node_data = match lab_node_data.iter().find(|n| n.name == nsd.name) {
            Some(n) if n.kind == data::NodeKind::Container => n,
            _ => continue,
        };

        // P2p containers handle ALL data interfaces via veth+netns move,
        // so they get no Docker network attachments for data interfaces.
        if p2p_container_names.contains(&nsd.name) {
            container_link_networks.insert(nsd.name.clone(), vec![]);
            continue;
        }

        let attachments = container_networks::data_attachments(
            &nsd.name,
            node_data.model,
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

    #[test]
    fn the_management_network_spans_its_subnets() {
        let subnets = LabSubnets {
            loopback_subnet: "127.127.1.0/24".parse().unwrap(),
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
    async fn a_second_up_of_a_lab_is_refused_before_it_creates_anything() {
        // M3.1: the second `up` used to fail in phase 3, and its rollback removed the first
        // `up`'s lab. Now the record is the reservation, taken before anything is created.
        let (db, alice) = database_with("alice").await;
        let details = db::LabDetails::default();
        let first = reserve_lab(&db, &config(), "core", "ab12cd34", &alice, &details)
            .await
            .unwrap();
        assert_eq!(first.record.phase, LabPhase::Creating);

        let error = reserve_lab(&db, &config(), "core", "ab12cd34", &alice, &details)
            .await
            .err()
            .unwrap();

        assert!(
            matches!(ServiceError::find(&error), Some(ServiceError::Conflict(_))),
            "{error:#}"
        );
        assert!(
            error.to_string().contains("already exists and is creating"),
            "{error}"
        );
        let lab = db::find_lab(&db, "ab12cd34").await.unwrap().unwrap();
        assert_eq!(lab.id, first.record.id);
        assert_eq!(lab.phase, LabPhase::Creating);
    }

    #[test]
    fn a_lab_id_another_lab_uses_is_reported_without_naming_it() {
        let alice = RecordId::new("user", "alice".to_string());
        let bob = RecordId::new("user", "bob".to_string());
        let lab = |name: &str, user: &RecordId, phase: LabPhase| data::DbLab {
            id: None,
            lab_id: "ab12cd34".to_string(),
            name: name.to_string(),
            user: user.clone(),
            loopback_network: String::new(),
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
            ssh_key: data::LabSshKey::Lab,
        };

        let redeployed = lab_id_taken(
            "ab12cd34",
            &lab("core", &alice, LabPhase::Ready),
            "core",
            Some(&alice),
        )
        .to_string();
        assert!(
            redeployed.contains("Lab already exists. Destroy the existing lab"),
            "{redeployed}"
        );

        let stranded = lab_id_taken(
            "ab12cd34",
            &lab("core", &alice, LabPhase::DestroyFailed),
            "core",
            Some(&alice),
        )
        .to_string();
        assert!(
            stranded.contains("already exists and is destroy failed"),
            "{stranded}"
        );

        let collision = lab_id_taken(
            "ab12cd34",
            &lab("x", &bob, LabPhase::Ready),
            "core",
            Some(&alice),
        )
        .to_string();
        assert!(collision.contains("rename the lab"), "{collision}");
        assert!(!collision.contains("'x'"), "{collision}");
    }
}
