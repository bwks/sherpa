// Server-side implementation of the lab startup operation
// This is a port of the client's up.rs command with streaming progress support

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use askama::Template;
use serde_json;

use opentelemetry::KeyValue;

use tracing::instrument;
use virt::storage_pool::StoragePool;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_starter;
use crate::daemon::state::AppState;
use crate::services::clean;
use crate::services::container_networks;
use crate::services::guest_account;
use crate::services::image_lifecycle::{lease_image, validate_disk_method};
use crate::services::lab_operations::{self, StopReason};
use crate::services::node_ops::{self, VmReadiness};
use crate::services::progress::ProgressSender;
use crate::services::talos;
use crate::services::{ssh_config, tailscale};
use crate::tls;

use crate::services::node_resources;
use crate::services::node_secrets::write_node_secrets;
use crate::validate::{
    reject_provisioners, validate_and_resolve_node_versions, validate_environment_variables,
    validate_node_secrets,
};
use shared::data;
use shared::data::{LabSubnets, LinkImpairment, NodeSshUsername, NodeState, StatusKind};
use shared::konst::{
    BRIDGE_MEMBER_HOST_VETH_PREFIX, BRIDGE_PREFIX, CONTAINER_DNSMASQ_CAPABILITIES,
    CONTAINER_DNSMASQ_NAME, CONTAINER_DNSMASQ_REPO, CONTAINER_SECRETS_DIR, DNSMASQ_CONFIG_FILE,
    DNSMASQ_DIR, DNSMASQ_LEASES_FILE, KVM_OUI, LAB_CA_CERT_FILE, LAB_CA_KEY_FILE,
    LAB_CERT_VALIDITY_DAYS, LAB_CERTS_DIR, LAB_FILE_NAME, LAB_SUBNET_ALLOCATION_ATTEMPTS,
    MANAGEMENT_GATEWAY_HOST_OFFSET, MANAGEMENT_NODE_HOST_OFFSETS, MTU_JUMBO_NET, NODE_CONFIGS_DIR,
    READINESS_SLEEP, READINESS_TIMEOUT, SHERPA_CONFIG_FILE_PATH, SHERPA_LAB_MANIFEST_FILE,
    SHERPA_LABS_PATH, SHERPA_LOOPBACK_PREFIX, SHERPA_LOOPBACK_PREFIX_IPV6,
    SHERPA_MANAGEMENT_IP_INDEX, SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX,
    SHERPA_MANAGEMENT_NETWORK_IPV6, SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_SECRETS_DIR,
    SHERPA_SSH_PRIVATE_KEY_PATH, SHERPA_STORAGE_POOL, SSH_PORT, TAP_PREFIX, TFTP_DIR, ZTP_DIR,
};
use shared::util;
use shared::validators::effective_data_interface_count;
use topology::find_talos_cluster;
use topology::validate::{
    check_bridge_device, check_device_count, check_duplicate_device,
    check_duplicate_interface_link, check_interface_bounds, check_link_device, check_mgmt_usage,
    check_no_p2p_field, validate_talos_clusters,
};

/// A data interface of a P2p container: a veth created into the container's netns once it has
/// started, with its host end named `host_veth`.
struct P2pContainerVeth {
    node_name: String,
    host_veth: String,
    host_alias: String,
    interface_idx: u8,
    node_model: data::NodeModel,
    admin_down: bool,
    /// The private bridge the host end joins, for a bridge member.
    bridge: Option<String>,
}

/// Whether a node counts as ready before readiness polling starts: one with
/// `skip_ready_check`, or a Talos cluster worker, which is checked as a Kubernetes node later.
/// Containers are started during polling, so a skipped container is handled when it starts.
fn skips_readiness_before_polling(
    skip_ready_check: bool,
    is_container: bool,
    cluster_worker: bool,
) -> bool {
    !is_container && (skip_ready_check || cluster_worker)
}

/// Attach the eBPF redirect, and any impairment, on each P2p link not yet in `attached` whose
/// ends both exist, by name in `ready`. Returns how many links it attached.
#[instrument(skip_all, fields(%lab_id), level = "debug")]
async fn attach_ready_p2p_links(
    lab_id: &str,
    p2p_links: &[&data::LabLinkData],
    links_detailed: &[topology::LinkDetailed],
    ready: &HashSet<String>,
    attached: &mut HashSet<u16>,
) -> Result<usize> {
    let mut count = 0;
    for link_data in p2p_links {
        if attached.contains(&link_data.index)
            || !ready.contains(&link_data.tap_a)
            || !ready.contains(&link_data.tap_b)
        {
            continue;
        }
        let tap_a = &link_data.tap_a;
        let tap_b = &link_data.tap_b;

        let ifindex_a = network::get_ifindex(tap_a)
            .await
            .context(format!("failed to get ifindex for {tap_a}"))?;
        let ifindex_b = network::get_ifindex(tap_b)
            .await
            .context(format!("failed to get ifindex for {tap_b}"))?;

        network::attach_p2p_redirect(tap_a, ifindex_b)
            .context(format!("failed to attach eBPF redirect on {tap_a}"))?;
        network::attach_p2p_redirect(tap_b, ifindex_a)
            .context(format!("failed to attach eBPF redirect on {tap_b}"))?;

        // Apply link impairment if configured
        let link_detail = links_detailed
            .iter()
            .find(|l| l.link_idx == link_data.index);
        if let Some(ld) = link_detail
            && let Some(ref impairment_cfg) = ld.impairment
        {
            let impairment = LinkImpairment::from(impairment_cfg);
            network::apply_link_netem(ifindex_a, ifindex_b, &impairment, &impairment).await?;
        }

        tracing::info!(
            lab_id = %lab_id,
            tap_a = %tap_a,
            tap_b = %tap_b,
            ifindex_a = ifindex_a,
            ifindex_b = ifindex_b,
            "Attached eBPF P2p redirect"
        );
        attached.insert(link_data.index);
        count += 1;
    }
    Ok(count)
}

// ============================================================================
// ============================================================================
// Helper Functions
// ============================================================================
// node_isolated_network_data, node_reserved_network_data, ztp_config_filename,
// take_custom_ztp_config, get_node_data — imported from node_ops module

fn find_interface_link(
    node_name: &str,
    interface_name: &str,
    links_detailed: &Vec<topology::LinkDetailed>,
) -> Option<data::NodeInterface> {
    let mut interface_data = None;
    for link in links_detailed {
        if link.node_a == node_name && link.int_a == *interface_name {
            interface_data = Some(data::NodeInterface::Peer(data::PeerInterface {
                link_index: link.link_idx,
                this_node: link.node_a.clone(),
                this_node_index: link.node_a_idx,
                this_interface: link.int_a.clone(),
                this_interface_index: link.int_a_idx,
                this_side: data::PeerSide::A,
                peer_node: link.node_b.clone(),
                peer_node_index: link.node_b_idx,
                peer_interface: link.int_b.clone(),
                peer_interface_index: link.int_b_idx,
                peer_side: data::PeerSide::B,
            }))
        } else if link.node_b == node_name && link.int_b == *interface_name {
            interface_data = Some(data::NodeInterface::Peer(data::PeerInterface {
                link_index: link.link_idx,
                this_node: link.node_b.clone(),
                this_node_index: link.node_b_idx,
                this_interface: link.int_b.clone(),
                this_interface_index: link.int_b_idx,
                this_side: data::PeerSide::B,
                peer_node: link.node_a.clone(),
                peer_node_index: link.node_a_idx,
                peer_interface: link.int_a.clone(),
                peer_interface_index: link.int_a_idx,
                peer_side: data::PeerSide::A,
            }))
        }
    }
    interface_data
}

fn find_bridge_interface(
    node_name: &str,
    interface_name: &str,
    bridge_connections: &[topology::BridgeDetailed],
) -> Option<data::NodeInterface> {
    let mut interface_data = None;
    for bridge in bridge_connections.iter() {
        for link in bridge.links.iter() {
            if link.node_name == node_name && link.interface_name == *interface_name {
                interface_data = Some(data::NodeInterface::Bridge(data::BridgeInterface {
                    name: bridge.bridge_name.clone(),
                }))
            }
        }
    }
    interface_data
}

fn process_manifest_bridges(
    manifest_bridges: &Option<Vec<topology::Bridge>>,
    manifest_nodes: &[topology::NodeExpanded],
    lab_id: &str,
) -> Result<Vec<topology::BridgeDetailed>> {
    let manifest_bridges = manifest_bridges.clone().unwrap_or_default();
    let bridges = manifest_bridges
        .iter()
        .map(|x: &topology::Bridge| x.parse_links())
        .collect::<Result<Vec<_>>>()?;

    let mut bridges_detailed = vec![];
    for (bridge_idx, bridge) in bridges.iter().enumerate() {
        let mut bridge_links = vec![];

        for link in bridge.links.iter() {
            if let Some(node) = manifest_nodes.iter().find(|n| n.name == link.node) {
                let interface_idx = util::interface_to_idx(&node.model, &link.interface)?;
                bridge_links.push(topology::BridgeLinkDetailed {
                    node_name: link.node.clone(),
                    node_model: node.model,
                    interface_name: link.interface.clone(),
                    interface_index: interface_idx,
                });
            }
        }

        bridges_detailed.push(topology::BridgeDetailed {
            manifest_name: bridge.name.clone(),
            libvirt_name: format!("sherpa-bridge{}-{}-{}", bridge_idx, bridge.name, lab_id),
            bridge_name: format!("{}s{}-{}", BRIDGE_PREFIX, bridge_idx, lab_id),
            index: bridge_idx as u16,
            links: bridge_links,
        });
    }
    Ok(bridges_detailed)
}

/// Process manifest nodes into expanded format with indices assigned
#[instrument(level = "debug", skip(manifest_nodes))]
pub(crate) fn process_manifest_nodes(
    manifest_nodes: &[topology::Node],
) -> Vec<topology::NodeExpanded> {
    manifest_nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| topology::NodeExpanded {
            name: node.name.clone(),
            model: node.model,
            index: idx as u16 + 1,
            version: node.version.clone(),
            memory: node.memory,
            cpu_count: node.cpu_count,
            boot_disk_size: node.boot_disk_size.or(node.model.default_boot_disk_size()),
            boot_disk_clone_method: node.boot_disk_clone_method,
            data_interface_count: node.data_interface_count,
            ipv4_address: None,
            ipv6_address: None,
            ssh_authorized_keys: node.ssh_authorized_keys.clone(),
            ssh_authorized_key_files: node.ssh_authorized_key_files.clone(),
            text_files: node.text_files_data.clone(),
            binary_files: node.binary_files.clone(),
            systemd_units: node.systemd_units.clone(),
            image: node.image.clone(),
            privileged: node.privileged,
            shm_size: node.shm_size,
            environment_variables: node.environment_variables.clone(),
            volumes: node.volumes.clone(),
            commands: node.commands.clone(),
            user: node.user.clone(),
            skip_ready_check: node.skip_ready_check,
            ztp_config: node.ztp_config.clone(),
            startup_scripts: node.startup_scripts_data.clone(),
            user_scripts: node.user_scripts_data.clone(),
            kernel_cmdline: node.kernel_cmdline.clone(),
            ready_port: node.ready_port,
        })
        .collect()
}

/// Process manifest links into detailed link format with resolved interface indices
fn process_manifest_links(
    manifest_links: &Option<Vec<topology::Link2>>,
    manifest_nodes: &[topology::NodeExpanded],
) -> Result<Vec<topology::LinkDetailed>> {
    let manifest_links = manifest_links.clone().unwrap_or_default();
    let links = manifest_links
        .iter()
        .map(|x: &topology::Link2| x.expand())
        .collect::<Result<Vec<topology::LinkExpanded>>>()?;

    let mut links_detailed = vec![];
    for (link_idx, link) in links.iter().enumerate() {
        let mut this_link = topology::LinkDetailed {
            impairment: link.impairment.clone(),
            ..Default::default()
        };
        for device in manifest_nodes.iter() {
            let device_model = device.model;
            if link.node_a == device.name {
                let int_idx = util::interface_to_idx(&device_model, &link.int_a)?;
                let peer_node = manifest_nodes
                    .iter()
                    .find(|n| n.name == link.node_b)
                    .ok_or_else(|| anyhow!("Peer node not found: {}", link.node_b))?;
                this_link.node_a = device.name.clone();
                this_link.node_a_idx = device.index;
                this_link.node_a_model = device_model;
                this_link.int_a = link.int_a.clone();
                this_link.int_a_idx = int_idx;
                this_link.link_idx = link_idx as u16;
                this_link.node_b_idx = peer_node.index;
            } else if link.node_b == device.name {
                let peer_node = manifest_nodes
                    .iter()
                    .find(|n| n.name == link.node_a)
                    .ok_or_else(|| anyhow!("Peer node not found: {}", link.node_a))?;
                let int_idx = util::interface_to_idx(&device_model, &link.int_b)?;
                this_link.node_b = device.name.clone();
                this_link.node_b_idx = device.index;
                this_link.node_b_model = device_model;
                this_link.int_b = link.int_b.clone();
                this_link.int_b_idx = int_idx;
                this_link.link_idx = link_idx as u16;
                this_link.node_a_idx = peer_node.index;
            }
        }
        links_detailed.push(this_link)
    }
    Ok(links_detailed)
}

/// Get node image from a list of node images.
/// When a version is provided, match on that version. Otherwise fall back to the default.
fn get_node_image(
    node_model: &data::NodeModel,
    version: Option<&str>,
    data: &[data::NodeConfig],
) -> Result<data::NodeConfig> {
    if let Some(v) = version {
        data.iter()
            .find(|x| &x.model == node_model && x.version == v)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "Node image not found for model: {} version: {}",
                    node_model,
                    v
                )
            })
    } else {
        data.iter()
            .find(|x| &x.model == node_model && x.default)
            .cloned()
            .ok_or_else(|| anyhow!("Default node image not found for model: {}", node_model))
    }
}

// ============================================================================
// Main Up Service Function
// ============================================================================

/// The lab record fields for a set of reserved subnets.
fn lab_networks(subnets: &LabSubnets) -> db::LabNetworks {
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
async fn allocate_lab_networks(db: &db::Database, config: &data::Config) -> Result<LabSubnets> {
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
async fn networks_taken(db: &db::Database, networks: &LabSubnets) -> Result<bool> {
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

/// Allocate subnets and create the lab record that holds them.
///
/// The record's unique subnet indexes are the reservation. When a concurrent `up` takes a
/// chosen subnet first, the insert fails, and allocation is repeated with fresh state.
/// Any other failure is returned as is.
#[instrument(skip(db, config, user), fields(%lab_id), level = "debug")]
async fn reserve_lab_networks(
    db: &db::Database,
    config: &data::Config,
    lab_name: &str,
    lab_id: &str,
    user: &data::DbUser,
) -> Result<(data::DbLab, LabSubnets)> {
    for attempt in 1..=LAB_SUBNET_ALLOCATION_ATTEMPTS {
        let networks = allocate_lab_networks(db, config).await?;
        match db::create_lab(db, lab_name, lab_id, user, &lab_networks(&networks)).await {
            Ok(lab) => {
                tracing::info!(
                    lab_id = %lab_id,
                    loopback_subnet = %networks.loopback_subnet,
                    management_subnet = %networks.management_subnet,
                    ipv6_management_subnet = %networks.ipv6_management_subnet,
                    ipv6_loopback_subnet = %networks.ipv6_loopback_subnet,
                    "Reserved lab subnets"
                );
                return Ok((lab, networks));
            }
            Err(error) => {
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

/// Start a lab with streaming progress updates
///
/// The lab is created for the authenticated caller. `request.username` is ignored. The built-in
/// `admin` account cannot start labs.
#[instrument(skip(ctx, request, state, progress), fields(username = %ctx.username, lab_id = %request.lab_id))]
pub async fn up_lab(
    ctx: &AuthContext,
    request: data::UpRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<data::UpResponse> {
    let start_time = Instant::now();
    let mut phases_completed = Vec::new();
    let mut errors = Vec::new();

    let lab_id = &request.lab_id;
    require_lab_starter(ctx)?;

    // Deserialize manifest from JSON Value
    let manifest: topology::Manifest =
        serde_json::from_value(request.manifest).context("Failed to deserialize manifest")?;
    reject_provisioners(&manifest).context("Manifest validation failed: provisioners")?;
    validate_node_secrets(&manifest, &request.node_secrets)
        .context("Manifest validation failed: container secret files")?;

    let tailnet_enabled = tailscale::validate_enrollment(
        &manifest,
        request.tailscale_auth_key.as_ref(),
        state.config.tls.enabled,
    )?;
    db::validate_lab_id(lab_id)?;
    // Destroying the lab stops this operation; see `lab_operations`.
    let operation = lab_operations::begin(lab_id).await;

    tracing::info!(
        "Starting lab creation: lab_id={}, name={}",
        lab_id,
        manifest.name
    );

    // ========================================================================
    // PHASE 1: Setup & Connections
    // ========================================================================
    let _ = progress.send_phase(data::UpPhase::Setup, "Initializing connections".to_string());

    tracing::info!(lab_id = %lab_id, lab_name = %manifest.name, "Connecting to lab infrastructure services");

    let sherpa_user = util::sherpa_user().context("Failed to get sherpa user")?;
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let current_user = &ctx.username;
    let management_network = format!("{}-{}", SHERPA_MANAGEMENT_NETWORK_NAME, lab_id);

    // Get connections from AppState
    let docker_conn = state.docker.clone();
    tracing::info!(lab_id = %lab_id, "Connected to Docker daemon");

    let qemu_conn = Arc::new(
        state
            .qemu
            .connect()
            .context("Failed to connect to libvirt")?,
    );
    tracing::info!(lab_id = %lab_id, "Connected to libvirt/QEMU");

    // Use the shared database connection from AppState
    let db = state.db.clone();
    tracing::info!(lab_id = %lab_id, "Connected to SurrealDB database");

    tracing::debug!(lab_id = %lab_id, lab_dir = %lab_dir, user = %current_user, "Lab environment prepared");

    let db_user = db::get_user(&db, current_user)
        .await
        .context("Failed to get database user")?;
    guest_account::require_ssh_key(&db_user)?;

    // Check if lab already exists (CRITICAL ERROR - fail fast)
    if let Ok(lab) = db::get_lab(&db, lab_id).await {
        bail!(
            "Lab already exists. Please use a different lab ID or destroy the existing lab first.\n Lab name: {}\n Lab id: {}",
            lab.name,
            lab_id,
        );
    }

    let _ = progress.send_status("Loading configuration".to_string(), StatusKind::Info);
    let config = state.config.clone();

    // Bulk fetch all node configs from database
    let node_images = db::list_node_images(&db)
        .await
        .context("Failed to list node configs from database")?;

    phases_completed.push("Setup".to_string());

    // ========================================================================
    // PHASE 2: Manifest Validation
    // ========================================================================
    let _ = progress.send_phase(
        data::UpPhase::ManifestValidation,
        "Validating manifest structure".to_string(),
    );

    tracing::info!(
        lab_id = %lab_id,
        lab_name = %manifest.name,
        "Validating lab manifest"
    );

    tracing::debug!(lab_id = %lab_id, node_images = node_images.len(), "Fetched node configs from database");

    // Device Validators (CRITICAL ERROR - fail fast on validation failure)
    check_duplicate_device(&manifest.nodes)
        .context("Manifest validation failed: duplicate devices")?;
    check_device_count(&manifest.nodes).context("Manifest validation failed: node count")?;
    validate_talos_clusters(&manifest).context("Manifest validation failed: talos clusters")?;

    // Environment variable validators
    for node in &manifest.nodes {
        if let Some(ref env_vars) = node.environment_variables {
            validate_environment_variables(env_vars, &node.name)
                .context("Manifest validation failed: environment variables")?;
        }
    }

    // Version & Image Validators (CRITICAL ERROR - fail fast on validation failure)
    // Fetch local Docker images for validation
    let docker_images = container::get_local_images(&docker_conn)
        .await
        .context("Failed to list local Docker images")?;

    let validated_nodes = validate_and_resolve_node_versions(
        &manifest.nodes,
        &node_images,
        &config.images_dir,
        &docker_images,
    )
    .context("Manifest validation failed: version/image validation")?;

    let nodes_expanded = process_manifest_nodes(&validated_nodes);
    // Acquire in a stable order to avoid deadlocks between overlapping labs.
    // Leases stop deletion racing validation, database references and disk creation.
    // Disk image (VM and unikernel) leases end once every disk and domain exists: node
    // records and overlay backing files protect the images from then on, and a long
    // readiness wait must not block image pull, import or delete. Container image leases
    // last until the containers start, during readiness polling.
    let mut selected_images = nodes_expanded
        .iter()
        .map(|node| get_node_image(&node.model, node.version.as_deref(), &node_images))
        .collect::<Result<Vec<_>>>()?;
    selected_images.sort_by_key(|image| (image.model.to_string(), image.version.clone()));
    selected_images.dedup_by(|a, b| a.model == b.model && a.version == b.version);
    let mut image_guards = Vec::new();
    let mut container_image_guards = Vec::new();
    for image in &selected_images {
        let lease = lease_image(image.model, &image.version).await;
        if image.kind == data::NodeKind::Container {
            container_image_guards.push(lease);
        } else {
            image_guards.push(lease);
        }
        let current = db::get_node_image_by_model_kind_version(
            &db,
            &image.model,
            &image.kind,
            &image.version,
        )
        .await?
        .ok_or_else(|| anyhow!("Selected image was deleted before provisioning"))?;
        anyhow::ensure!(
            current.id == image.id,
            "Selected image changed before provisioning; retry startup"
        );
    }
    for node in &nodes_expanded {
        let image = get_node_image(&node.model, node.version.as_deref(), &node_images)?;
        validate_disk_method(
            node.boot_disk_clone_method,
            &image,
            &config.images_dir,
            node.boot_disk_size,
        )
        .with_context(|| format!("Disk provisioning validation failed for node {}", node.name))?;
    }

    let mut personal_nodes = Vec::new();
    if manifest.provision_current_user {
        for node in &nodes_expanded {
            let image = get_node_image(&node.model, node.version.as_deref(), &node_images)?;
            guest_account::validate_node(node, &image)?;
            if guest_account::supports_guest_account(&image) {
                personal_nodes.push(node.name.clone());
            }
        }
    }
    let personal_account = if personal_nodes.is_empty() {
        None
    } else {
        let account = guest_account::from_user(&db_user)?;
        if account.is_none() {
            let _ = progress.send_status(
                format!(
                    "Username '{}' cannot be a login on lab nodes; nodes use the sherpa account",
                    db_user.username
                ),
                StatusKind::Info,
            );
        }
        account
    };
    let mut ssh_usernames = Vec::new();
    if let Some(account) = &personal_account {
        for name in personal_nodes {
            ssh_usernames.push(NodeSshUsername {
                node_name: name,
                username: account.username.clone(),
            });
        }
    }

    check_no_p2p_field(manifest.links.as_deref().unwrap_or_default())
        .context("Link validation failed")?;
    let links_detailed = process_manifest_links(&manifest.links, &nodes_expanded)
        .context("Failed to process manifest links")?;
    let bridges_detailed = process_manifest_bridges(&manifest.bridges, &nodes_expanded, lab_id)
        .context("Failed to process manifest bridges")?;

    tracing::info!(
        lab_id = %lab_id,
        nodes = nodes_expanded.len(),
        links = links_detailed.len(),
        bridges = bridges_detailed.len(),
        "Processed manifest structures"
    );

    let mut ztp_records = vec![];

    for node in &nodes_expanded {
        let node_image = get_node_image(&node.model, node.version.as_deref(), &node_images)
            .context(format!("Node config not found for model: {}", node.model))?;

        if !node_image.dedicated_management_interface {
            check_mgmt_usage(&node.name, 0, &links_detailed, &bridges_detailed).context(
                format!(
                    "Management interface validation failed for node: {}",
                    node.name
                ),
            )?;
        }

        let data_interface_count =
            effective_data_interface_count(&node.name, node.data_interface_count, &node_image)
                .context(format!(
                    "Data interface count validation failed for node: {}",
                    node.name
                ))?;

        check_interface_bounds(
            &node.name,
            &node_image.model,
            data_interface_count,
            node_image.reserved_interface_count,
            node_image.dedicated_management_interface,
            &links_detailed,
            &bridges_detailed,
        )
        .context(format!(
            "Interface bounds validation failed for node: {}",
            node.name
        ))?;
    }

    // Connection Validators
    if !links_detailed.is_empty() {
        check_duplicate_interface_link(&links_detailed, &bridges_detailed)
            .context("Duplicate interface link validation failed")?;
        check_link_device(&manifest.nodes, &links_detailed)
            .context("Link device validation failed")?;
    }

    // Bridge Validators
    if !bridges_detailed.is_empty() {
        check_bridge_device(&manifest.nodes, &bridges_detailed)
            .context("Bridge device validation failed")?;
    }

    let _ = progress.send_status("Manifest validation complete".to_string(), StatusKind::Done);
    tracing::info!(lab_id = %lab_id, "Manifest validation completed successfully");
    phases_completed.push("ManifestValidation".to_string());

    // Startup rollback must never remove resources from a previous attempt.
    if selected_images
        .iter()
        .any(|image| image.kind != data::NodeKind::Container)
    {
        let pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)?;
        libvirt::refresh_pool(&pool)?;
        anyhow::ensure!(
            !pool
                .list_volumes()?
                .iter()
                .any(|name| name.contains(lab_id)),
            "Existing disks for this lab ID must be cleaned before startup"
        );
        for domain in qemu_conn.list_all_domains(0)? {
            anyhow::ensure!(
                !domain.get_name()?.contains(lab_id),
                "Existing VM for this lab ID must be cleaned before startup"
            );
        }
    }

    // Wrap resource-creating phases in a block so we can auto-clean on failure.
    // Phases 1-2 (Setup, ManifestValidation) don't create resources, so failures
    // there propagate directly without cleanup.
    let resource_creation = async {
        // ========================================================================
        // PHASE 3: Database Records & Data Structure Building
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::DatabaseRecords,
            "Creating database records".to_string(),
        );

        tracing::info!(lab_id = %lab_id, lab_name = %manifest.name, "Creating database records");

        let (lab_record, networks) =
            reserve_lab_networks(&db, &config, &manifest.name, lab_id, &db_user).await?;
        let LabSubnets {
            loopback_subnet,
            management_subnet,
            gateway_ipv4,
            router_ipv4,
            ipv6_management_subnet,
            gateway_ipv6,
            router_ipv6,
            ..
        } = networks;

        let lab_record_id = db::get_lab_id(&lab_record).context("Failed to get lab record ID")?;

        tracing::info!(lab_id = %lab_id, lab_name = %manifest.name, "Created lab database record");

        let mut container_nodes: Vec<topology::NodeExpanded> = vec![];
        let mut unikernel_nodes: Vec<topology::NodeExpanded> = vec![];
        let mut vm_nodes: Vec<topology::NodeExpanded> = vec![];
        let mut clone_disks: Vec<data::CloneDisk> = vec![];
        let mut domains: Vec<template::DomainTemplate> = vec![];
        let mut unikernel_domains: Vec<template::UnikernelDomainTemplate> = vec![];

        let mut lab_node_data = vec![];
        let mut node_setup_data = vec![];

        for node in nodes_expanded.iter() {
            let node_image = get_node_image(&node.model, node.version.as_deref(), &node_images)?;

            tracing::info!(
                lab_id = %lab_id,
                node_name = %node.name,
                node_kind = ?node_image.kind,
                node_model = ?node_image.model,
                "Creating node database record"
            );

            // Build interface data structures
            let mut node_interfaces_detailed: Vec<data::InterfaceData> = vec![];
            let data_interface_count =
                effective_data_interface_count(&node.name, node.data_interface_count, &node_image)?;
            let first_data_interface_idx = node_image
                .reserved_interface_count
                .checked_add(1)
                .ok_or_else(|| {
                    anyhow!("Reserved interface count overflow for node {}", node.name)
                })?;
            let max_interface_idx = node_image
                .reserved_interface_count
                .checked_add(data_interface_count)
                .ok_or_else(|| anyhow!("Data interface count overflow for node {}", node.name))?;

            for idx in 0..=max_interface_idx {
                let interface_name = util::interface_from_idx(&node.model, idx)?;
                let interface_idx = idx;
                let mut interface_state = data::InterfaceState::Enabled;
                let mut interface_data = data::NodeInterface::Disabled;

                if idx == 0 {
                    interface_data = data::NodeInterface::Management;
                } else if idx < first_data_interface_idx {
                    interface_data = data::NodeInterface::Reserved;
                } else {
                    if let Some(data) =
                        find_interface_link(&node.name, &interface_name, &links_detailed)
                    {
                        interface_data = data
                    }
                    if let Some(data) =
                        find_bridge_interface(&node.name, &interface_name, &bridges_detailed)
                    {
                        interface_data = data
                    }
                    interface_state = data::InterfaceState::Disabled;
                }

                node_interfaces_detailed.push(data::InterfaceData {
                    name: interface_name.to_string(),
                    index: interface_idx,
                    state: interface_state,
                    data: interface_data,
                });
            }

            let lab_node = db::create_node(
                &db,
                &node.name,
                node.index,
                db::get_image_id(&node_image)?,
                lab_record_id.clone(),
            )
            .await?;
            // Record its vCPUs, memory and, for a VM, its VNC console port.
            let lab_node = node_resources::reserve_node_resources(
                &db,
                &lab_node,
                &node_image.kind,
                node_resources::planned_resources(node, &node_image),
            )
            .await?;

            lab_node_data.push(data::LabNodeData {
                name: node.name.clone(),
                model: node_image.model,
                kind: node_image.kind.clone(),
                index: node.index,
                record: lab_node,
            });

            match node_image.kind {
                data::NodeKind::Container => {
                    container_nodes.push(node.clone());
                }
                data::NodeKind::Unikernel => {
                    unikernel_nodes.push(node.clone());
                }
                data::NodeKind::VirtualMachine => {
                    vm_nodes.push(node.clone());
                }
            }

            let has_disabled_interfaces = node_interfaces_detailed
                .iter()
                .any(|i| matches!(i.data, data::NodeInterface::Disabled));

            let node_isolated_network = if matches!(
                node_image.kind,
                data::NodeKind::VirtualMachine | data::NodeKind::Container
            ) && has_disabled_interfaces
            {
                Some(node_ops::node_isolated_network_data(
                    &node.name, node.index, lab_id,
                ))
            } else {
                None
            };

            let node_reserved_network = if matches!(node_image.kind, data::NodeKind::VirtualMachine)
                && node_image.reserved_interface_count > 0
            {
                Some(node_ops::node_reserved_network_data(
                    &node.name, node.index, lab_id,
                ))
            } else {
                None
            };

            if let Some(network) = node_isolated_network.clone() {
                let _ = progress.send_status(
                    format!("Creating isolated network for node: {}", node.name),
                    StatusKind::Progress,
                );
                tracing::info!(
                    lab_id = %lab_id,
                    node_name = %node.name,
                    network_type = "isolated",
                    "Creating node isolated network"
                );
                match node_image.kind {
                    data::NodeKind::VirtualMachine => {
                        let node_isolated_network = libvirt::IsolatedNetwork {
                            network_name: network.network_name,
                            bridge_name: network.bridge_name,
                        };
                        node_isolated_network.create(&qemu_conn)?;
                    }
                    data::NodeKind::Container => {
                        network::create_bridge(&network.bridge_name, &network.network_name).await?;
                    }
                    data::NodeKind::Unikernel => {
                        tracing::warn!(
                            node_name = %node.name,
                            "Skipping isolated network creation for unikernel node"
                        );
                    }
                }
            }

            if let Some(network) = node_reserved_network.clone() {
                let _ = progress.send_status(
                    format!("Creating reserved network for node: {}", node.name),
                    StatusKind::Progress,
                );
                tracing::info!(
                    lab_id = %lab_id,
                    node_name = %node.name,
                    network_type = "reserved",
                    "Creating node reserved network"
                );
                let node_reserved_network = libvirt::ReservedNetwork {
                    network_name: network.network_name,
                    bridge_name: network.bridge_name,
                };
                node_reserved_network.create(&qemu_conn)?;
            }

            node_setup_data.push(data::NodeSetupData {
                name: node.name.clone(),
                index: node.index,
                management_network: management_network.clone(),
                isolated_network: node_isolated_network,
                reserved_network: node_reserved_network,
                interfaces: node_interfaces_detailed,
            });
        }

        tracing::info!(
            lab_id = %lab_id,
            total_nodes = nodes_expanded.len(),
            containers = container_nodes.len(),
            vms = vm_nodes.len(),
            unikernels = unikernel_nodes.len(),
            "Created all node database records"
        );

        phases_completed.push("DatabaseRecords".to_string());

        // ========================================================================
        // PHASE 4: Lab Network Setup
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::LabNetworkSetup,
            "Allocating lab network and creating management network".to_string(),
        );

        let lab_net = management_subnet;

        tracing::info!(
            lab_id = %lab_id,
            subnet = %lab_net,
            gateway = %gateway_ipv4,
            boot_server = %router_ipv4,
            "Allocated lab network subnet"
        );

        let lab_info = data::LabInfo {
            ssh_usernames: ssh_usernames.clone(),
            id: lab_id.to_string(),
            user: current_user.clone(),
            name: manifest.name.clone(),
            ipv4_network: lab_net,
            ipv4_gateway: gateway_ipv4,
            ipv4_router: router_ipv4,
            loopback_network: loopback_subnet,
            ipv6_network: Some(ipv6_management_subnet),
            ipv6_gateway: Some(gateway_ipv6),
            ipv6_router: Some(router_ipv6),
        };

        util::create_dir(&lab_dir)?;
        util::create_file(&format!("{lab_dir}/{LAB_FILE_NAME}"), lab_info.to_string())?;

        // Save the manifest for future redeploy operations
        let manifest_toml =
            toml::to_string_pretty(&manifest).context("Failed to serialize manifest for saving")?;
        let manifest_path = format!("{lab_dir}/{SHERPA_LAB_MANIFEST_FILE}");
        util::create_file(&manifest_path, manifest_toml)?;
        util::set_file_permissions(&manifest_path, 0o600)?;

        let mgmt_net = data::SherpaNetwork {
            v4: data::NetworkV4 {
                prefix: lab_net,
                first: gateway_ipv4,
                last: lab_net.broadcast(),
                boot_server: router_ipv4,
                network: lab_net.network(),
                subnet_mask: lab_net.netmask(),
                hostmask: lab_net.hostmask(),
                prefix_length: lab_net.prefix_len(),
            },
            v6: Some(data::NetworkV6 {
                prefix: ipv6_management_subnet,
                first: gateway_ipv6,
                last: util::get_ipv6_addr(&ipv6_management_subnet, u32::MAX)?,
                boot_server: router_ipv6,
                network: ipv6_management_subnet.network(),
                prefix_length: ipv6_management_subnet.prefix_len(),
            }),
        };
        let dns = util::default_dns_dual_stack(&lab_net, &ipv6_management_subnet)?;

        let _ = progress.send_status(
            format!("Creating management network: {SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            StatusKind::Progress,
        );

        tracing::info!(
            lab_id = %lab_id,
            network = %format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            "Creating management network"
        );

        // Libvirt management network
        let management_network_obj = libvirt::NatNetwork {
            network_name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            bridge_name: format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
            ipv4_address: gateway_ipv4,
            ipv4_netmask: lab_net.netmask(),
            ipv6_address: Some(gateway_ipv6),
            ipv6_prefix_length: Some(ipv6_management_subnet.prefix_len()),
        };
        management_network_obj.create(&qemu_conn)?;

        tracing::info!(
            lab_id = %lab_id,
            network = %format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            bridge = %format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
            "Created libvirt NAT network"
        );

        // Docker management network
        container::create_docker_bridge_network(
            &docker_conn,
            &format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            Some(lab_net.to_string()),
            Some(ipv6_management_subnet.to_string()),
            &format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{lab_id}"),
        )
        .await?;

        tracing::info!(
            lab_id = %lab_id,
            network = %format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            "Created Docker bridge network"
        );

        let tailnet_status = if tailnet_enabled {
            let _ = progress.send_status(
                "Connecting lab to Tailscale".to_string(),
                StatusKind::Progress,
            );
            let key = request
                .tailscale_auth_key
                .as_ref()
                .context("Missing Tailscale enrollment credential")?;
            let status = tailscale::provision(state, &lab_record, &lab_info, key).await?;
            for warning in &status.warnings {
                let _ = progress.send_status(format!("Warning: {warning}"), StatusKind::Info);
                errors.push(data::UpError {
                    phase: "Tailscale".into(),
                    message: warning.clone(),
                    is_critical: false,
                });
            }
            Some(status)
        } else {
            None
        };

        phases_completed.push("LabNetworkSetup".to_string());

        // ========================================================================
        // PHASE 5: Point-to-Point Link Creation
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::LinkCreation,
            format!("Creating {} point-to-point links", links_detailed.len()),
        );

        tracing::info!(
            lab_id = %lab_id,
            link_count = links_detailed.len(),
            "Creating point-to-point links"
        );

        let mut lab_link_data = vec![];

        let mut p2p_container_veths: Vec<P2pContainerVeth> = vec![];
        // Track which container nodes participate in P2p links.
        // These containers handle ALL data interfaces via veth+netns, not Docker networks.
        let mut p2p_container_names: HashSet<String> = HashSet::new();

        for (idx, link) in links_detailed.iter().enumerate() {
            let node_a = lab_node_data
                .iter()
                .find(|n| n.name == link.node_a)
                .ok_or_else(|| anyhow!("Node not found: {}", link.node_a))?;

            let node_b = lab_node_data
                .iter()
                .find(|n| n.name == link.node_b)
                .ok_or_else(|| anyhow!("Node not found: {}", link.node_b))?;

            let link_kind = data::BridgeKind::P2p;

            let tap_a = format!("{}a{}-{}", TAP_PREFIX, link.link_idx, lab_id);
            let tap_b = format!("{}b{}-{}", TAP_PREFIX, link.link_idx, lab_id);

            // Create the link in the database
            let mut db_link = db::create_link(
                &db,
                link.link_idx,
                link_kind.clone(),
                db::get_node_id(&node_a.record)?,
                db::get_node_id(&node_b.record)?,
                link.int_a.clone(),
                link.int_b.clone(),
                tap_a.clone(),
                tap_b.clone(),
                lab_record_id.clone(),
            )
            .await?;
            // The manifest's impairment applies to both directions.
            if let Some(impairment) = &link.impairment {
                db_link.impairment_a_to_b = LinkImpairment::from(impairment);
                db_link.impairment_b_to_a = LinkImpairment::from(impairment);
                db::update_link(&db, db_link).await.context(format!(
                    "failed to store impairment of link {}",
                    link.link_idx
                ))?;
            }

            lab_link_data.push(data::LabLinkData {
                index: link.link_idx,
                kind: link_kind.clone(),
                node_a: node_a.record.clone(),
                node_b: node_b.record.clone(),
                int_a: link.int_a.clone(),
                int_b: link.int_b.clone(),
                tap_a: tap_a.clone(),
                tap_b: tap_b.clone(),
            });

            let _ = progress.send_status(
                format!(
                    "Creating link #{} ({}) - {}::{} <-> {}::{}",
                    idx, link_kind, link.node_a, link.int_a, link.node_b, link.int_b
                ),
                StatusKind::Progress,
            );

            tracing::info!(
                lab_id = %lab_id,
                link_num = idx,
                link_kind = %link_kind,
                node_a = %link.node_a,
                int_a = %link.int_a,
                node_b = %link.node_b,
                int_b = %link.int_b,
                "Creating point-to-point link"
            );

            // P2p link: for VMs, libvirt creates the tap devices via type='ethernet'.
            // A container end is a veth created into the container's netns once it has
            // started, its host end named as the tap. eBPF redirect is attached once both
            // ends exist.

            if node_a.kind == data::NodeKind::Container {
                p2p_container_veths.push(P2pContainerVeth {
                    node_name: link.node_a.clone(),
                    host_veth: tap_a.clone(),
                    host_alias: format!("{}-p2p-host-{}::{}", lab_id, link.node_a, link.int_a),
                    interface_idx: link.int_a_idx,
                    node_model: link.node_a_model,
                    admin_down: false,
                    bridge: None,
                });
                p2p_container_names.insert(link.node_a.clone());
            }

            if node_b.kind == data::NodeKind::Container {
                p2p_container_veths.push(P2pContainerVeth {
                    node_name: link.node_b.clone(),
                    host_veth: tap_b.clone(),
                    host_alias: format!("{}-p2p-host-{}::{}", lab_id, link.node_b, link.int_b),
                    interface_idx: link.int_b_idx,
                    node_model: link.node_b_model,
                    admin_down: false,
                    bridge: None,
                });
                p2p_container_names.insert(link.node_b.clone());
            }

            tracing::info!(
                lab_id = %lab_id,
                tap_a = %tap_a,
                tap_b = %tap_b,
                "P2p link registered — eBPF redirect will attach after node creation"
            );
        }

        tracing::info!(
            lab_id = %lab_id,
            links_created = links_detailed.len(),
            "All point-to-point links created"
        );

        // Record veths for disabled interfaces on P2p containers.
        // P2p containers handle ALL data interfaces via veth+netns (not Docker networks)
        // to avoid ethN naming conflicts between Docker and the container's init process.
        for nsd in &node_setup_data {
            if !p2p_container_names.contains(&nsd.name) {
                continue;
            }
            let node_data = lab_node_data
                .iter()
                .find(|n| n.name == nsd.name)
                .ok_or_else(|| anyhow!("P2p container {} not found in lab_node_data", nsd.name))?;

            for iface in &nsd.interfaces {
                if !matches!(iface.data, data::NodeInterface::Disabled) {
                    continue;
                }
                // A disabled interface's veth stays down, with no eBPF.
                // Format: cd{node_idx}i{iface_idx}-{lab_id}, compact to stay within Linux's
                // 15-char interface name limit.
                p2p_container_veths.push(P2pContainerVeth {
                    node_name: nsd.name.clone(),
                    host_veth: format!("cd{}i{}-{}", nsd.index, iface.index, lab_id),
                    host_alias: format!(
                        "{}-p2p-disabled-host-{}::{}",
                        lab_id, nsd.name, iface.name
                    ),
                    interface_idx: iface.index,
                    node_model: node_data.model,
                    admin_down: true,
                    bridge: None,
                });
            }
        }

        phases_completed.push("LinkCreation".to_string());

        // ========================================================================
        // PHASE 6: Docker networks for disabled interfaces of containers with no links
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::ContainerNetworks,
            "Creating Docker networks for disabled container interfaces".to_string(),
        );

        tracing::info!(lab_id = %lab_id, "Creating Docker networks for disabled container interfaces");

        let mut docker_net_count = 0;
        // Create Docker macvlan bridge-mode networks for disabled interfaces on container nodes
        for nsd in &node_setup_data {
            let iso_network = match &nsd.isolated_network {
                Some(n) => n,
                None => continue,
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
                    &docker_conn,
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

        phases_completed.push("ContainerNetworks".to_string());

        // ========================================================================
        // PHASE 7: Shared Bridge Creation
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::SharedBridges,
            format!("Creating {} shared bridges", bridges_detailed.len()),
        );

        tracing::info!(
            lab_id = %lab_id,
            bridge_count = bridges_detailed.len(),
            "Creating shared bridges"
        );

        for bridge in bridges_detailed.iter() {
            let mut bridge_members = vec![];

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

            for link in bridge.links.iter() {
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
                        &docker_conn,
                        &bridge.bridge_name,
                        &docker_net_name,
                    )
                    .await?;
                }
            }

            db::create_bridge(
                &db,
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

        phases_completed.push("SharedBridges".to_string());

        // ========================================================================
        // PHASE 8: ZTP Configuration Generation
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::ZtpGeneration,
            "Generating ZTP configurations".to_string(),
        );

        tracing::info!(
            lab_id = %lab_id,
            total_nodes = container_nodes.len() + vm_nodes.len(),
            "Generating ZTP configurations"
        );

        // Create ZTP directories for TFTP-based ZTP (needed before VM ZTP generation)
        let ztp_dir = format!("{lab_dir}/{ZTP_DIR}");
        let tftp_dir = format!("{ztp_dir}/{TFTP_DIR}");
        util::create_dir(&ztp_dir)?;
        util::create_dir(&tftp_dir)?;

        // Generate per-lab TLS CA certificate
        let certs_dir = format!("{lab_dir}/{LAB_CERTS_DIR}");
        util::create_dir(&certs_dir)?;

        let ca_cert_path = Path::new(&certs_dir).join(LAB_CA_CERT_FILE);
        let ca_key_path = Path::new(&certs_dir).join(LAB_CA_KEY_FILE);

        let lab_ca = tls::generator::generate_lab_ca(
            &ca_cert_path,
            &ca_key_path,
            lab_id,
            LAB_CERT_VALIDITY_DAYS,
        )
        .context("Failed to generate lab CA certificate")?;

        let _ = progress.send_status(
            format!("Lab CA certificate generated for lab: {}", lab_id),
            StatusKind::Done,
        );

        // Container nodes ZTP generation
        for node in &mut container_nodes {
            // Decode base64 ztp_config if present
            if let Some(ref encoded) = node.ztp_config {
                let decoded = util::base64_decode(encoded).with_context(|| {
                    format!("Failed to decode ztp_config for node '{}'", node.name)
                })?;
                node.ztp_config = Some(decoded);
            }

            let node_data = node_ops::get_node_data(&node.name, &node_setup_data)?;
            let node_idx = node_data.index;
            let node_ip_idx = util::node_management_host_offset(node_idx)?;

            let node_image = get_node_image(&node.model, node.version.as_deref(), &node_images)?;
            let node_ipv4_address = util::get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?;
            node.ipv4_address = Some(node_ipv4_address);

            // Assign IPv6 management address
            if let Some(ref v6) = mgmt_net.v6 {
                let addr = util::get_ipv6_addr(&v6.prefix, node_ip_idx)?;
                node.ipv6_address = Some(addr);
            }

            // Persist management IPs to the database
            if let Some(node_data) = lab_node_data.iter().find(|n| n.name == node.name) {
                let record_id = db::get_node_id(&node_data.record)?;
                db::update_node_mgmt_ipv4(&db, record_id.clone(), &node_ipv4_address.to_string())
                    .await?;
                if let Some(ipv6) = node.ipv6_address {
                    db::update_node_mgmt_ipv6(&db, record_id, &ipv6.to_string()).await?;
                }
            }

            // Generate per-node TLS certificate
            let node_cert_path = Path::new(&certs_dir).join(format!("{}.crt", node.name));
            let node_key_path = Path::new(&certs_dir).join(format!("{}.key", node.name));
            tls::generator::generate_node_certificate(
                &node_cert_path,
                &node_key_path,
                &lab_ca,
                &node.name,
                &node_ipv4_address.to_string(),
                LAB_CERT_VALIDITY_DAYS,
            )
            .with_context(|| {
                format!(
                    "Failed to generate TLS certificate for node '{}'",
                    node.name
                )
            })?;

            let cert_paths = node_ops::NodeCertPaths {
                ca_cert: ca_cert_path.to_string_lossy().to_string(),
                node_cert: node_cert_path.to_string_lossy().to_string(),
                node_key: node_key_path.to_string_lossy().to_string(),
            };

            let ztp_result = node_ops::generate_container_ztp(
                node,
                &node_image,
                &lab_dir,
                &sherpa_user,
                &dns,
                &mgmt_net,
                node_ipv4_address,
                &progress,
                Some(&cert_paths),
            )?;

            ztp_records.push(ztp_result.ztp_record);
        }

        // VM nodes ZTP generation, disk setup, and domain template building
        tracing::info!(
            lab_id = %lab_id,
            vm_count = vm_nodes.len(),
            "Generating VM ZTP configurations and domain templates"
        );

        // Plan Kubernetes clusters and generate their secrets before any node config.
        let talos_clusters = manifest.talos_clusters.as_deref().unwrap_or_default();
        let mut cluster_members = Vec::new();
        let mut cluster_names = Vec::with_capacity(talos_clusters.len());
        for cluster in talos_clusters {
            cluster_names.push(cluster.name.as_str());
            for (name, role) in cluster.members() {
                let node = vm_nodes
                    .iter()
                    .find(|node| node.name == name)
                    .with_context(|| format!("Talos cluster member '{name}' is not a VM node"))?;
                let image = get_node_image(&node.model, node.version.as_deref(), &node_images)?;
                talos::secrets::ensure_supported_version(name, &image.version)?;
                let node_idx = node_ops::get_node_data(name, &node_setup_data)?.index;
                cluster_members.push(talos::plan::MemberInput {
                    name,
                    cluster: &cluster.name,
                    role,
                    ipv4: util::get_ipv4_addr(
                        &mgmt_net.v4.prefix,
                        util::node_management_host_offset(node_idx)?,
                    )?,
                });
            }
        }
        let lab_clusters = if cluster_names.is_empty() {
            Vec::new()
        } else {
            let clusters =
                talos::prepare_clusters(lab_id, &cluster_names, &cluster_members, &mgmt_net.v4)?;
            let _ = progress.send_status(
                format!(
                    "Generated secrets for {} Kubernetes clusters",
                    clusters.len()
                ),
                StatusKind::Done,
            );
            clusters
        };

        for node in &mut vm_nodes {
            // Decode base64 ztp_config if present
            if let Some(ref encoded) = node.ztp_config {
                let decoded = util::base64_decode(encoded).with_context(|| {
                    format!("Failed to decode ztp_config for node '{}'", node.name)
                })?;
                node.ztp_config = Some(decoded);
            }
            // Cluster nodes get a generated machine config (validation forbids their own).
            if let Some((cluster, _)) = find_talos_cluster(talos_clusters, &node.name) {
                let cluster = talos::find_cluster(&lab_clusters, &cluster.name)?;
                node.ztp_config = Some(talos::node_machine_config(
                    cluster,
                    &node.name,
                    &mgmt_net.v4,
                )?);
            }

            let node_data = node_ops::get_node_data(&node.name, &node_setup_data)?;
            let node_idx = node_data.index;
            let node_ip_idx = util::node_management_host_offset(node_idx)?;

            let node_image = get_node_image(&node.model, node.version.as_deref(), &node_images)?;
            let node_ipv4_address = util::get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?;
            node.ipv4_address = Some(node_ipv4_address);

            // Assign IPv6 management address
            if let Some(ref v6) = mgmt_net.v6 {
                let addr = util::get_ipv6_addr(&v6.prefix, node_ip_idx)?;
                node.ipv6_address = Some(addr);
            }

            // Persist management IPs to the database
            if let Some(node_data) = lab_node_data.iter().find(|n| n.name == node.name) {
                let record_id = db::get_node_id(&node_data.record)?;
                db::update_node_mgmt_ipv4(&db, record_id.clone(), &node_ipv4_address.to_string())
                    .await?;
                if let Some(ipv6) = node.ipv6_address {
                    db::update_node_mgmt_ipv6(&db, record_id, &ipv6.to_string()).await?;
                }
            }

            // Generate per-node TLS certificate
            let node_cert_path = Path::new(&certs_dir).join(format!("{}.crt", node.name));
            let node_key_path = Path::new(&certs_dir).join(format!("{}.key", node.name));
            tls::generator::generate_node_certificate(
                &node_cert_path,
                &node_key_path,
                &lab_ca,
                &node.name,
                &node_ipv4_address.to_string(),
                LAB_CERT_VALIDITY_DAYS,
            )
            .with_context(|| {
                format!(
                    "Failed to generate TLS certificate for node '{}'",
                    node.name
                )
            })?;

            let cert_paths = node_ops::NodeCertPaths {
                ca_cert: ca_cert_path.to_string_lossy().to_string(),
                node_cert: node_cert_path.to_string_lossy().to_string(),
                node_key: node_key_path.to_string_lossy().to_string(),
            };

            // Generate VM ZTP configuration, disks, and clone list
            let ztp_result = node_ops::generate_vm_ztp(
                node,
                &node_image,
                lab_id,
                &lab_dir,
                &tftp_dir,
                &config.images_dir,
                &mgmt_net,
                node_ipv4_address,
                &sherpa_user,
                &dns,
                &progress,
                None,
                Some(&cert_paths),
                personal_account.as_ref(),
            )?;

            // Persist management MAC to the database
            if let Some(node_data) = lab_node_data.iter().find(|n| n.name == node.name) {
                let record_id = db::get_node_id(&node_data.record)?;
                db::update_node_mgmt_mac(&db, record_id, &ztp_result.mac_address).await?;
            }

            ztp_records.push(ztp_result.ztp_record);
            clone_disks.extend(ztp_result.clone_disks);

            // Build interfaces list
            let mut interfaces: Vec<data::Interface> = vec![];
            for interface in node_data.interfaces.iter() {
                match &interface.data {
                    data::NodeInterface::Management => {
                        interfaces.push(data::Interface {
                            name: util::dasher(&node_image.management_interface.to_string()),
                            num: interface.index,
                            mtu: node_image.interface_mtu,
                            mac_address: ztp_result.mac_address.clone(),
                            connection_type: data::ConnectionTypes::Management,
                        });
                    }
                    data::NodeInterface::Reserved => {
                        interfaces.push(data::Interface {
                            name: format!("int{}", interface.index),
                            num: interface.index,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::Reserved,
                        });
                    }
                    data::NodeInterface::Bridge(bridge) => {
                        interfaces.push(data::Interface {
                            name: bridge.name.clone(),
                            num: interface.index,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::PrivateBridge,
                        });
                    }
                    data::NodeInterface::Peer(peer) => {
                        let tap_name = match peer.this_side {
                            data::PeerSide::A => {
                                format!("{}a{}-{}", TAP_PREFIX, peer.link_index, lab_id)
                            }
                            data::PeerSide::B => {
                                format!("{}b{}-{}", TAP_PREFIX, peer.link_index, lab_id)
                            }
                        };
                        interfaces.push(data::Interface {
                            name: tap_name,
                            num: interface.index,
                            mtu: MTU_JUMBO_NET,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::P2p,
                        });
                    }
                    data::NodeInterface::Disabled => {
                        interfaces.push(data::Interface {
                            name: util::dasher(&util::interface_from_idx(
                                &node.model,
                                interface.index,
                            )?),
                            num: interface.index,
                            mtu: node_image.interface_mtu,
                            mac_address: util::random_mac(KVM_OUI),
                            connection_type: data::ConnectionTypes::Disabled,
                        });
                    }
                }
            }

            // Get network names
            let management_network = node_data.management_network.clone();
            let isolated_network_name = node_data
                .isolated_network
                .as_ref()
                .map(|net| net.network_name.clone())
                .unwrap_or_default();
            let reserved_network = node_data
                .reserved_network
                .as_ref()
                .map(|net| net.network_name.clone())
                .unwrap_or_default();

            // Build domain template from the resources stored on the node
            let record = lab_node_data
                .iter()
                .find(|n| n.name == node.name)
                .map(|n| &n.record)
                .with_context(|| format!("Node '{}' has no database record", node.name))?;
            let domain = node_ops::build_domain_template(
                node,
                &node_image,
                lab_id,
                &config.qemu_bin,
                ztp_result.disks,
                interfaces,
                ztp_result.qemu_commands,
                util::get_ip(&loopback_subnet, node_idx as u8).to_string(),
                management_network,
                isolated_network_name,
                reserved_network,
                &node_resources::resources_of(record),
            )?;
            domains.push(domain);
        }

        // Unikernel nodes: IP allocation, TLS certs, interface building, setup, and domain template
        if !unikernel_nodes.is_empty() {
            tracing::info!(
                lab_id = %lab_id,
                unikernel_count = unikernel_nodes.len(),
                "Processing unikernel nodes"
            );

            for node in &mut unikernel_nodes {
                let node_data = node_ops::get_node_data(&node.name, &node_setup_data)?;
                let node_idx = node_data.index;
                let node_ip_idx = util::node_management_host_offset(node_idx)?;

                let node_image =
                    get_node_image(&node.model, node.version.as_deref(), &node_images)?;
                let node_ipv4_address = util::get_ipv4_addr(&mgmt_net.v4.prefix, node_ip_idx)?;
                node.ipv4_address = Some(node_ipv4_address);

                // Assign IPv6 management address
                if let Some(ref v6) = mgmt_net.v6 {
                    let addr = util::get_ipv6_addr(&v6.prefix, node_ip_idx)?;
                    node.ipv6_address = Some(addr);
                }

                // Persist management IPs to the database
                if let Some(node_data) = lab_node_data.iter().find(|n| n.name == node.name) {
                    let record_id = db::get_node_id(&node_data.record)?;
                    db::update_node_mgmt_ipv4(
                        &db,
                        record_id.clone(),
                        &node_ipv4_address.to_string(),
                    )
                    .await?;
                    if let Some(ipv6) = node.ipv6_address {
                        db::update_node_mgmt_ipv6(&db, record_id, &ipv6.to_string()).await?;
                    }
                }

                // Generate per-node TLS certificate
                let node_cert_path = Path::new(&certs_dir).join(format!("{}.crt", node.name));
                let node_key_path = Path::new(&certs_dir).join(format!("{}.key", node.name));
                tls::generator::generate_node_certificate(
                    &node_cert_path,
                    &node_key_path,
                    &lab_ca,
                    &node.name,
                    &node_ipv4_address.to_string(),
                    LAB_CERT_VALIDITY_DAYS,
                )
                .with_context(|| {
                    format!(
                        "Failed to generate TLS certificate for unikernel node '{}'",
                        node.name
                    )
                })?;

                // Generate unikernel setup (disk clone or kernel path)
                let setup_result = node_ops::generate_unikernel_setup(
                    node,
                    &node_image,
                    lab_id,
                    &config.images_dir,
                )?;

                // Persist management MAC to the database
                if let Some(node_data) = lab_node_data.iter().find(|n| n.name == node.name) {
                    let record_id = db::get_node_id(&node_data.record)?;
                    db::update_node_mgmt_mac(&db, record_id, &setup_result.mac_address).await?;
                }

                // Create ZTP record for DHCP static binding (MAC -> IP)
                // Unikernels use ZtpMethod::None — no boot file, just the DHCP host entry
                ztp_records.push(data::ZtpRecord {
                    node_name: node.name.clone(),
                    config_file: String::new(),
                    ipv4_address: node_ipv4_address,
                    ipv6_address: node.ipv6_address,
                    mac_address: setup_result.mac_address.clone(),
                    ztp_method: data::ZtpMethod::None,
                    ssh_port: Some(SSH_PORT),
                });

                clone_disks.extend(setup_result.clone_disks);

                // Build interfaces list (same pattern as VMs)
                let mut interfaces: Vec<data::Interface> = vec![];
                for interface in node_data.interfaces.iter() {
                    match &interface.data {
                        data::NodeInterface::Management => {
                            interfaces.push(data::Interface {
                                name: util::dasher(&node_image.management_interface.to_string()),
                                num: interface.index,
                                mtu: node_image.interface_mtu,
                                mac_address: setup_result.mac_address.clone(),
                                connection_type: data::ConnectionTypes::Management,
                            });
                        }
                        data::NodeInterface::Reserved => {
                            interfaces.push(data::Interface {
                                name: format!("int{}", interface.index),
                                num: interface.index,
                                mtu: node_image.interface_mtu,
                                mac_address: util::random_mac(KVM_OUI),
                                connection_type: data::ConnectionTypes::Reserved,
                            });
                        }
                        data::NodeInterface::Bridge(bridge) => {
                            interfaces.push(data::Interface {
                                name: bridge.name.clone(),
                                num: interface.index,
                                mtu: node_image.interface_mtu,
                                mac_address: util::random_mac(KVM_OUI),
                                connection_type: data::ConnectionTypes::PrivateBridge,
                            });
                        }
                        data::NodeInterface::Peer(peer) => {
                            let tap_name = match peer.this_side {
                                data::PeerSide::A => {
                                    format!("{}a{}-{}", TAP_PREFIX, peer.link_index, lab_id)
                                }
                                data::PeerSide::B => {
                                    format!("{}b{}-{}", TAP_PREFIX, peer.link_index, lab_id)
                                }
                            };
                            interfaces.push(data::Interface {
                                name: tap_name,
                                num: interface.index,
                                mtu: MTU_JUMBO_NET,
                                mac_address: util::random_mac(KVM_OUI),
                                connection_type: data::ConnectionTypes::P2p,
                            });
                        }
                        data::NodeInterface::Disabled => {
                            interfaces.push(data::Interface {
                                name: util::dasher(&util::interface_from_idx(
                                    &node.model,
                                    interface.index,
                                )?),
                                num: interface.index,
                                mtu: node_image.interface_mtu,
                                mac_address: util::random_mac(KVM_OUI),
                                connection_type: data::ConnectionTypes::Disabled,
                            });
                        }
                    }
                }

                // Get network names
                let management_network = node_data.management_network.clone();
                let isolated_network_name = node_data
                    .isolated_network
                    .as_ref()
                    .map(|net| net.network_name.clone())
                    .unwrap_or_default();
                let reserved_network = node_data
                    .reserved_network
                    .as_ref()
                    .map(|net| net.network_name.clone())
                    .unwrap_or_default();

                // Build unikernel domain template with auto IP injection
                let domain = node_ops::build_unikernel_domain_template(
                    node,
                    &node_image,
                    lab_id,
                    &config.qemu_bin,
                    setup_result.disks,
                    interfaces,
                    setup_result.kernel_path,
                    util::get_ip(&loopback_subnet, node_idx as u8).to_string(),
                    management_network,
                    isolated_network_name,
                    reserved_network,
                    &mgmt_net,
                );
                unikernel_domains.push(domain);

                let _ = progress.send_status(
                    format!("Unikernel node {} configured", node.name),
                    StatusKind::Done,
                );
            }
        }

        phases_completed.push("ZtpGeneration".to_string());

        // ========================================================================
        // PHASE 9: Sherpa Router & ZTP File Creation
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::BootContainers,
            "Creating boot containers and ZTP files".to_string(),
        );

        tracing::info!(lab_id = %lab_id, "Creating ZTP boot infrastructure");

        // Create remaining ZTP directories (ztp_dir and tftp_dir already created in Phase 8)
        let ztp_configs_dir = format!("{ztp_dir}/{NODE_CONFIGS_DIR}");
        let dnsmasq_dir = format!("{ztp_dir}/{DNSMASQ_DIR}");
        util::create_dir(&ztp_configs_dir)?;
        util::create_dir(&dnsmasq_dir)?;

        tracing::debug!(
            lab_id = %lab_id,
            ztp_dir = %ztp_dir,
            tftp_dir = %tftp_dir,
            dnsmasq_dir = %dnsmasq_dir,
            "Created ZTP directories"
        );

        // Create dnsmasq config
        let dnsmaq_template = template::DnsmasqTemplate {
            tftp_server_ipv4: mgmt_net.v4.boot_server.to_string(),
            gateway_ipv4: mgmt_net.v4.first.to_string(),
            dhcp_start: util::get_ipv4_addr(
                &mgmt_net.v4.prefix,
                *MANAGEMENT_NODE_HOST_OFFSETS.start(),
            )?
            .to_string(),
            dhcp_end: util::get_ipv4_addr(
                &mgmt_net.v4.prefix,
                *MANAGEMENT_NODE_HOST_OFFSETS.end(),
            )?
            .to_string(),
            gateway_ipv6: mgmt_net.v6.as_ref().map(|v6| v6.first.to_string()),
            dhcp6_start: mgmt_net
                .v6
                .as_ref()
                .map(|v6| {
                    util::get_ipv6_addr(&v6.prefix, *MANAGEMENT_NODE_HOST_OFFSETS.start())
                        .map(|a| a.to_string())
                })
                .transpose()?,
            dhcp6_end: mgmt_net
                .v6
                .as_ref()
                .map(|v6| {
                    util::get_ipv6_addr(&v6.prefix, *MANAGEMENT_NODE_HOST_OFFSETS.end())
                        .map(|a| a.to_string())
                })
                .transpose()?,
            dns_ipv6: mgmt_net.v6.as_ref().map(|v6| v6.boot_server.to_string()),
            ztp_records: ztp_records.clone(),
        };
        let dnsmasq_rendered_template = dnsmaq_template.render()?;
        util::create_file(
            &format!("{dnsmasq_dir}/{DNSMASQ_CONFIG_FILE}"),
            dnsmasq_rendered_template,
        )?;
        util::create_file(
            &format!("{dnsmasq_dir}/{DNSMASQ_LEASES_FILE}"),
            "".to_string(),
        )?;

        // Create boot container (tftp_dir already created in Phase 8)
        let configs_dir = format!("{ztp_dir}/{NODE_CONFIGS_DIR}");

        let dnsmasq_env_dns1 = format!("DNS1={}", mgmt_net.v4.first);
        let dnsmasq_env_dns2 = "DNS2=".to_string();
        let boot_server_ipv4 = mgmt_net.v4.boot_server.to_string();

        let webdir_config_volume = format!("{configs_dir}:/opt/{ZTP_DIR}/{NODE_CONFIGS_DIR}");
        let dnsmasq_env_vars = vec![dnsmasq_env_dns1, dnsmasq_env_dns2];
        let dnsmasq_config_volume =
            format!("{dnsmasq_dir}/{DNSMASQ_CONFIG_FILE}:/etc/{DNSMASQ_CONFIG_FILE}");
        let dnsmasq_tftp_volume = format!("{tftp_dir}:/opt/{ZTP_DIR}/{TFTP_DIR}");
        let dnsmasq_volumes = vec![
            dnsmasq_config_volume,
            dnsmasq_tftp_volume,
            webdir_config_volume,
        ];
        let dnsmasq_capabilities: Vec<String> = CONTAINER_DNSMASQ_CAPABILITIES
            .iter()
            .map(|s| s.to_string())
            .collect();

        let management_network_attachment = data::ContainerNetworkAttachment {
            name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            ipv4_address: Some(boot_server_ipv4.clone()),
            ipv6_address: None,
            linux_interface_name: None,
            admin_down: false,
        };

        tracing::info!(
            lab_id = %lab_id,
            container = %format!("{CONTAINER_DNSMASQ_NAME}-{lab_id}"),
            boot_server_ip = %boot_server_ipv4,
            "Starting dnsmasq boot container"
        );

        let is_running = container::run_container(
            &docker_conn,
            &format!("{CONTAINER_DNSMASQ_NAME}-{lab_id}"),
            CONTAINER_DNSMASQ_REPO,
            dnsmasq_env_vars,
            dnsmasq_volumes,
            dnsmasq_capabilities,
            management_network_attachment,
            vec![],
            vec![],
            false,
            None,
            None,
        )
        .await?;

        if !is_running {
            anyhow::bail!(
                "dnsmasq boot container {CONTAINER_DNSMASQ_NAME}-{lab_id} is not in running state after start"
            );
        }

        phases_completed.push("Sherpa Router".to_string());

        // ========================================================================
        // PHASE 10: Disk Cloning (For VMs and Unikernels)
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(data::UpPhase::DiskCloning, "Cloning disks".to_string());

        node_ops::clone_node_disks(
            qemu_conn.clone(),
            clone_disks,
            lab_id,
            &progress,
            image_guards.clone(),
        )
        .await?;

        phases_completed.push("DiskCloning".to_string());

        // ========================================================================
        // PHASE 11: VM Creation
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(data::UpPhase::VmCreation, "Creating VMs".to_string());

        if !domains.is_empty() {
            let vm_count = domains.len();
            let _ = progress.send_status(
                format!("Creating {} VMs in parallel", vm_count),
                StatusKind::Progress,
            );

            let tasks: Vec<_> = domains
                .into_iter()
                .map(|domain| {
                    let conn = Arc::clone(&qemu_conn);
                    let progress_clone = progress.clone();
                    tokio::task::spawn(async move {
                        node_ops::create_vm(conn, domain, &progress_clone).await
                    })
                })
                .collect();

            for task in tasks {
                task.await.context("VM creation task failed")??;
            }

            let _ =
                progress.send_status("All VMs created successfully".to_string(), StatusKind::Done);
        } else {
            let _ = progress.send_status("No VMs to create".to_string(), StatusKind::Info);
        }

        phases_completed.push("VmCreation".to_string());

        // Unikernel creation (uses same libvirt API as VMs)
        if !unikernel_domains.is_empty() {
            let uk_count = unikernel_domains.len();
            let _ = progress.send_status(
                format!("Creating {} unikernels in parallel", uk_count),
                StatusKind::Progress,
            );

            let tasks: Vec<_> = unikernel_domains
                .into_iter()
                .map(|domain| {
                    let conn = Arc::clone(&qemu_conn);
                    let progress_clone = progress.clone();
                    tokio::task::spawn(async move {
                        node_ops::create_unikernel(conn, domain, &progress_clone).await
                    })
                })
                .collect();

            for task in tasks {
                task.await.context("Unikernel creation task failed")??;
            }

            let _ = progress.send_status(
                "All unikernels created successfully".to_string(),
                StatusKind::Done,
            );
        }
        // Every disk and domain exists; stop holding the disk images.
        image_guards.clear();

        // ========================================================================
        // PHASE 11b: Attach eBPF redirect on P2p links
        // ========================================================================
        // Now that libvirt has created the tap devices, attach eBPF redirect programs to wire
        // up P2p links between VMs. A link with a container end is attached once that
        // container has started and its veth exists (see readiness polling).
        let p2p_links: Vec<&data::LabLinkData> = lab_link_data
            .iter()
            .filter(|l| l.kind == data::BridgeKind::P2p)
            .collect();
        let container_host_veths: HashSet<&str> = p2p_container_veths
            .iter()
            .map(|v| v.host_veth.as_str())
            .collect();
        let mut p2p_ready_ends: HashSet<String> = HashSet::new();
        for link_data in &p2p_links {
            for tap in [&link_data.tap_a, &link_data.tap_b] {
                if !container_host_veths.contains(tap.as_str()) {
                    p2p_ready_ends.insert(tap.clone());
                }
            }
        }
        let mut p2p_attached_links: HashSet<u16> = HashSet::new();

        if !p2p_links.is_empty() {
            let attached = attach_ready_p2p_links(
                lab_id,
                &p2p_links,
                &links_detailed,
                &p2p_ready_ends,
                &mut p2p_attached_links,
            )
            .await?;
            let _ = progress.send_status(
                format!("Attached eBPF redirect on {attached} P2p links"),
                StatusKind::Done,
            );
        }

        phases_completed.push("P2pEbpfAttach".to_string());

        // ========================================================================
        // PHASE 11c: Set isolated bridges DOWN
        // ========================================================================
        // Set isolated network bridges DOWN to remove carrier from disabled
        // interfaces, so they show as "not connected" on the node. Containers
        // attach to their bridge later, without running anything in the image.
        for node in vm_nodes
            .iter()
            .chain(unikernel_nodes.iter())
            .chain(container_nodes.iter())
        {
            let nsd = node_setup_data.iter().find(|n| n.name == node.name);
            if let Some(nsd) = nsd
                && let Some(ref iso_net) = nsd.isolated_network
            {
                tracing::info!(
                    lab_id = %lab_id,
                    node_name = %node.name,
                    bridge_name = %iso_net.bridge_name,
                    "Setting isolated bridge DOWN to remove carrier from disabled interfaces"
                );
                network::set_link_down(&iso_net.bridge_name).await?;
            }
        }

        // ========================================================================
        // PHASE 12: SSH Config & Network Map Building
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::SshConfig,
            "Generating SSH config".to_string(),
        );

        tracing::info!(lab_id = %lab_id, "Generating SSH configuration");

        // Load server config to get server_ipv4
        let config_contents = util::load_file(SHERPA_CONFIG_FILE_PATH)
            .context("Failed to load sherpa.toml config")?;
        let config: data::Config =
            toml::from_str(&config_contents).context("Failed to parse sherpa.toml config")?;

        tracing::debug!(
            lab_id = %lab_id,
            proxy_user = %lab_info.user,
            server_ip = %config.server_ipv4,
            "SSH config parameters"
        );

        // Generate the SSH config with ProxyJump. Node host keys are added once nodes are
        // ready, and the files saved again.
        let ssh_files = ssh_config::save_lab_ssh_files(
            &lab_dir,
            &ztp_records,
            &lab_info,
            &[],
            config.server_ipv4,
        )?;
        let _ = progress.send_status("SSH config file created".to_string(), StatusKind::Done);

        // Read SSH private key for transfer to client
        let ssh_private_key = if ssh_files.uses_lab_key {
            util::load_file(SHERPA_SSH_PRIVATE_KEY_PATH)
                .context("Failed to read SSH private key")?
        } else {
            String::new()
        };
        tracing::debug!(
            lab_id = %lab_id,
            key_path = %SHERPA_SSH_PRIVATE_KEY_PATH,
            "Loaded SSH private key"
        );
        let _ = progress.send_status("SSH private key loaded".to_string(), StatusKind::Done);

        // Build container network mappings from the full interface list.
        // This ensures all defined data interfaces appear in model-index order,
        // with linked interfaces active and disabled interfaces admin-down.
        let mut container_link_networks: HashMap<String, Vec<data::ContainerNetworkAttachment>> =
            HashMap::new();

        // Walk each container node's interfaces in index order.
        // For each data interface, produce a ContainerNetworkAttachment with
        // the correct docker network name and linux_interface_name.
        for nsd in &node_setup_data {
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
                &node_data.model,
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

        phases_completed.push("SshConfig".to_string());

        // ========================================================================
        // PHASE 13: Node Readiness Polling
        // ========================================================================
        let ready_timeout_secs = manifest.ready_timeout.unwrap_or(READINESS_TIMEOUT);
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::NodeReadiness,
            format!(
                "Waiting for {} nodes to become ready (up to {} seconds)",
                container_nodes.len() + vm_nodes.len(),
                ready_timeout_secs
            ),
        );

        let start_time_readiness = Instant::now();
        let readiness_timer = std::time::Instant::now();
        let timeout = Duration::from_secs(ready_timeout_secs);
        let mut connected_nodes = HashSet::new();
        let mut node_info_list = vec![];

        let all_lab_nodes = [
            container_nodes.clone(),
            unikernel_nodes.clone(),
            vm_nodes.clone(),
        ]
        .concat();
        let total_lab_nodes = all_lab_nodes.len();

        let node_names = all_lab_nodes
            .iter()
            .map(|x| x.name.as_str())
            .collect::<Vec<&str>>()
            .join(", ");

        tracing::debug!(
            lab_id = %lab_id,
            nodes = %node_names,
            "Waiting for nodes"
        );

        let _ = progress.send_status(
            format!("Waiting for nodes: {}", node_names),
            StatusKind::Waiting,
        );

        // Handle nodes with skip_ready_check enabled, and Talos cluster workers, which are
        // checked as Kubernetes nodes once their cluster is bootstrapped.
        let talos_clusters = manifest.talos_clusters.as_deref().unwrap_or_default();
        for node in &all_lab_nodes {
            let cluster_worker = talos::readiness_waits_for_kubernetes(talos_clusters, &node.name);
            let is_container = container_nodes.iter().any(|c| c.name == node.name);
            if skips_readiness_before_polling(
                node.skip_ready_check.unwrap_or(false),
                is_container,
                cluster_worker,
            ) {
                tracing::info!(
                    lab_id = %lab_id,
                    node_name = %node.name,
                    cluster_worker,
                    "Skipping ready check for node"
                );
                let message = if cluster_worker {
                    format!(
                        "Node {} - Ready check deferred to its Kubernetes cluster",
                        node.name
                    )
                } else {
                    format!("Node {} - Ready check skipped", node.name)
                };
                let _ = progress.send_status(message, StatusKind::Done);
                connected_nodes.insert(node.name.clone());
                let kind = lab_node_data
                    .iter()
                    .find(|n| n.name == node.name)
                    .map(|n| format!("{:?}", n.kind))
                    .unwrap_or_else(|| "Unknown".to_string());
                node_info_list.push(data::NodeInfo {
                    name: node.name.clone(),
                    kind,
                    model: node.model,
                    status: NodeState::Unknown,
                    ip_address: node.ipv4_address.map(|i| i.to_string()),
                    ssh_port: None,
                });
            }
        }

        tracing::info!(
            lab_id = %lab_id,
            total_nodes = total_lab_nodes,
            containers = container_nodes.len(),
            vms = vm_nodes.len(),
            unikernels = unikernel_nodes.len(),
            timeout_secs = ready_timeout_secs,
            "Starting node readiness polling"
        );

        // Secret files are written once, to RAM, and mounted when each container starts.
        let mut secret_mounts = HashMap::new();
        for (node_name, files) in &request.node_secrets {
            let dir =
                write_node_secrets(Path::new(SHERPA_SECRETS_DIR), lab_id, node_name, files)
                    .with_context(|| format!("Cannot write secret files for node '{node_name}'"))?;
            let _ = progress.send_status(
                format!(
                    "Wrote {} secret files for container: {node_name}",
                    files.len()
                ),
                StatusKind::Progress,
            );
            secret_mounts.insert(
                node_name.as_str(),
                format!("{}:{CONTAINER_SECRETS_DIR}:ro", dir.display()),
            );
        }

        while start_time_readiness.elapsed() < timeout && connected_nodes.len() < total_lab_nodes {
            // Start containers
            for container in &container_nodes {
                if connected_nodes.contains(&container.name) {
                    continue;
                }

                let mgmt_ipv4 = container.ipv4_address.map(|i| i.to_string());
                let container_name = format!("{}-{}", container.name, lab_id);

                // Extract image and version with proper error handling
                let container_image_name = container.image.as_ref().ok_or_else(|| {
                    anyhow!("Container image not set for node: {}", container.name)
                })?;

                let container_version = container.version.as_ref().ok_or_else(|| {
                    anyhow!("Container version not set for node: {}", container.name)
                })?;

                let container_image = format!("{}:{}", container_image_name, container_version);
                let privileged = container.privileged.unwrap_or(false);
                let shm_size = container.shm_size;
                let env_vars = container.environment_variables.clone().unwrap_or_default();
                let commands = container.commands.clone().unwrap_or_default();
                let user = container.user.clone();
                let mut volumes: Vec<String> = if let Some(volumes) = container.volumes.clone() {
                    volumes
                        .iter()
                        .map(|v| format!("{}:{}", v.src, v.dst))
                        .collect()
                } else {
                    vec![]
                };
                if let Some(mount) = secret_mounts.get(container.name.as_str()) {
                    volumes.push(mount.clone());
                }

                let management_network_attachment = data::ContainerNetworkAttachment {
                    name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
                    ipv4_address: mgmt_ipv4.clone(),
                    ipv6_address: None,
                    linux_interface_name: None,
                    admin_down: false,
                };

                let mut additional_networks = vec![];
                if let Some(link_networks) = container_link_networks.get(&container.name) {
                    additional_networks.extend_from_slice(link_networks);
                }

                let capabilities = node_ops::model_capabilities(&container.model);
                let is_running = node_ops::start_container_node(
                    &docker_conn,
                    &container_name,
                    &container_image,
                    env_vars,
                    volumes,
                    capabilities,
                    management_network_attachment,
                    additional_networks,
                    commands,
                    privileged,
                    shm_size,
                    user,
                    container.model,
                    &progress,
                )
                .await?;

                if !is_running {
                    // A skipped container is started once and not waited for.
                    if container.skip_ready_check.unwrap_or(false) {
                        tracing::info!(
                            lab_id = %lab_id,
                            node_name = %container.name,
                            "Container not running; ready check skipped"
                        );
                        let _ = progress.send_status(
                            format!("Node {} - Not running, ready check skipped", container.name),
                            StatusKind::Done,
                        );
                        connected_nodes.insert(container.name.clone());
                        node_info_list.push(data::NodeInfo {
                            name: container.name.clone(),
                            kind: "Container".to_string(),
                            model: container.model,
                            status: NodeState::Unknown,
                            ip_address: mgmt_ipv4,
                            ssh_port: None,
                        });
                    }
                    continue;
                }

                // Create the P2p container's data interfaces into its network namespace, named
                // and up, so nothing runs in the image.
                let container_p2p_veths: Vec<&P2pContainerVeth> = p2p_container_veths
                    .iter()
                    .filter(|v| v.node_name == container.name)
                    .collect();

                if !container_p2p_veths.is_empty() {
                    let pid = container::get_container_pid(&docker_conn, &container_name).await?;

                    for veth_info in &container_p2p_veths {
                        let target_name = if veth_info.node_model == data::NodeModel::NokiaSrlinux {
                            let iface_name = util::interface_from_idx(
                                &veth_info.node_model,
                                veth_info.interface_idx,
                            )?;
                            util::srlinux_to_linux_interface(&iface_name)?
                        } else {
                            format!("eth{}", veth_info.interface_idx)
                        };

                        network::create_veth_in_netns(
                            &veth_info.host_veth,
                            &veth_info.host_alias,
                            &target_name,
                            pid,
                            !veth_info.admin_down,
                        )
                        .await
                        .with_context(|| {
                            format!(
                                "Failed to create P2p interface {target_name} in container \
                                 {container_name}"
                            )
                        })?;
                        if let Some(bridge_name) = &veth_info.bridge {
                            network::enslave_to_bridge(&veth_info.host_veth, bridge_name).await?;
                        }
                        p2p_ready_ends.insert(veth_info.host_veth.clone());

                        tracing::info!(
                            lab_id = %lab_id,
                            container = %container_name,
                            host_veth = %veth_info.host_veth,
                            target = %target_name,
                            admin_down = veth_info.admin_down,
                            pid = pid,
                            "Created P2p veth into container netns"
                        );
                    }

                    attach_ready_p2p_links(
                        lab_id,
                        &p2p_links,
                        &links_detailed,
                        &p2p_ready_ends,
                        &mut p2p_attached_links,
                    )
                    .await?;
                }

                connected_nodes.insert(container.name.clone());

                // Update node state in DB to Running
                if let Some(node_data) = lab_node_data.iter().find(|n| n.name == container.name) {
                    let record_id = db::get_node_id(&node_data.record)?;
                    db::update_node_state(&db, record_id, NodeState::Running).await?;
                }

                node_info_list.push(data::NodeInfo {
                    name: container.name.clone(),
                    kind: "Container".to_string(),
                    model: container.model,
                    status: NodeState::Running,
                    ip_address: mgmt_ipv4,
                    ssh_port: Some(SSH_PORT),
                });
            }

            // Check VMs for readiness
            for vm in &vm_nodes {
                if connected_nodes.contains(&vm.name) {
                    continue;
                }

                if let Some(vm_data) = ztp_records.iter().find(|x| x.node_name == vm.name) {
                    let ready_port = node_ops::vm_ready_port(&vm.model, vm.ready_port);
                    let ready_label = node_ops::ready_port_label(ready_port);
                    match node_ops::check_vm_ready(&vm_data.ipv4_address.to_string(), ready_port)
                        .await?
                    {
                        VmReadiness::Ready { host_key } => {
                            tracing::info!(
                                lab_id = %lab_id,
                                node_name = %vm.name,
                                node_kind = "VirtualMachine",
                                ipv4 = %vm_data.ipv4_address,
                                ready_port,
                                "VM ready"
                            );
                            let _ = progress.send_status(
                                format!("Node {} - Ready ({ready_label} reachable)", vm.name),
                                StatusKind::Done,
                            );
                            connected_nodes.insert(vm.name.clone());

                            // Update node state in DB to Running, with the host key it offered
                            if let Some(node_data) =
                                lab_node_data.iter().find(|n| n.name == vm.name)
                            {
                                let record_id = db::get_node_id(&node_data.record)?;
                                if host_key.is_some() {
                                    db::update_node_host_key(
                                        &db,
                                        record_id.clone(),
                                        host_key.as_deref(),
                                    )
                                    .await?;
                                }
                                db::update_node_state(&db, record_id, NodeState::Running).await?;
                            }

                            node_info_list.push(data::NodeInfo {
                                name: vm.name.clone(),
                                kind: "VirtualMachine".to_string(),
                                model: vm.model,
                                status: NodeState::Running,
                                ip_address: Some(vm_data.ipv4_address.to_string()),
                                ssh_port: vm.model.has_ssh().then_some(SSH_PORT),
                            });
                        }
                        VmReadiness::NotReady => {
                            tracing::debug!(
                                lab_id = %lab_id,
                                node_name = %vm.name,
                                ipv4 = %vm_data.ipv4_address,
                                ready_port,
                                "Waiting for VM readiness port"
                            );
                            let _ = progress.send_status(
                                format!("Node {} - Waiting for {ready_label}", vm.name),
                                StatusKind::Waiting,
                            );
                        }
                    }
                }
            }

            // Check unikernels for readiness
            for uk in &unikernel_nodes {
                if connected_nodes.contains(&uk.name) {
                    continue;
                }

                let domain_name = format!("{}-{}", uk.name, lab_id);
                let mgmt_ip = uk.ipv4_address.map(|a| a.to_string());

                match node_ops::check_unikernel_ready(
                    &qemu_conn,
                    &domain_name,
                    uk.ready_port,
                    mgmt_ip.as_deref(),
                )? {
                    true => {
                        tracing::info!(
                            lab_id = %lab_id,
                            node_name = %uk.name,
                            node_kind = "Unikernel",
                            "Unikernel ready"
                        );
                        let _ = progress.send_status(
                            format!("Node {} - Ready (Unikernel running)", uk.name),
                            StatusKind::Done,
                        );
                        connected_nodes.insert(uk.name.clone());

                        // Update node state in DB to Running
                        if let Some(node_data) = lab_node_data.iter().find(|n| n.name == uk.name) {
                            let record_id = db::get_node_id(&node_data.record)?;
                            db::update_node_state(&db, record_id, NodeState::Running).await?;
                        }

                        node_info_list.push(data::NodeInfo {
                            name: uk.name.clone(),
                            kind: "Unikernel".to_string(),
                            model: uk.model,
                            status: NodeState::Running,
                            ip_address: mgmt_ip,
                            ssh_port: None,
                        });
                    }
                    false => {
                        tracing::debug!(
                            lab_id = %lab_id,
                            node_name = %uk.name,
                            "Waiting for unikernel to become ready"
                        );
                        let _ = progress.send_status(
                            format!("Node {} - Waiting for unikernel", uk.name),
                            StatusKind::Waiting,
                        );
                    }
                }
            }

            if connected_nodes.len() < total_lab_nodes {
                operation
                    .sleep(Duration::from_secs(READINESS_SLEEP))
                    .await?;
            }
        }

        for link_data in &p2p_links {
            if !p2p_attached_links.contains(&link_data.index) {
                tracing::warn!(
                    lab_id = %lab_id,
                    tap_a = %link_data.tap_a,
                    tap_b = %link_data.tap_b,
                    "P2p link not attached: one of its containers did not start"
                );
            }
        }

        let readiness_elapsed = readiness_timer.elapsed().as_secs();

        if connected_nodes.len() == total_lab_nodes {
            tracing::info!(
                lab_id = %lab_id,
                nodes_ready = connected_nodes.len(),
                total_nodes = total_lab_nodes,
                duration_secs = readiness_elapsed,
                "All nodes ready"
            );
            let _ = progress.send_status("All nodes are ready!".to_string(), StatusKind::Done);
        } else {
            tracing::warn!(
                lab_id = %lab_id,
                nodes_ready = connected_nodes.len(),
                total_nodes = total_lab_nodes,
                duration_secs = readiness_elapsed,
                timeout_secs = ready_timeout_secs,
                "Timeout reached - not all nodes ready"
            );
            let _ = progress.send_status(
                format!(
                    "Timeout reached. {} of {} nodes are ready.",
                    connected_nodes.len(),
                    total_lab_nodes
                ),
                StatusKind::Waiting,
            );
            for node in &all_lab_nodes {
                if !connected_nodes.contains(&node.name) {
                    tracing::warn!(
                        lab_id = %lab_id,
                        node_name = %node.name,
                        "Node not ready after timeout"
                    );
                    errors.push(data::UpError {
                        phase: "NodeReadiness".to_string(),
                        message: format!("Node {} did not become ready", node.name),
                        is_critical: false,
                    });
                }
            }
        }

        // Save the SSH files again with the host keys recorded as nodes became ready.
        let ssh_files = ssh_config::refresh_lab_ssh_files(
            &db,
            &lab_dir,
            &lab_record_id,
            &lab_info,
            config.server_ipv4,
        )
        .await?;

        // Bootstrap Kubernetes clusters once their nodes answer.
        let clusters = if lab_clusters.is_empty() {
            Vec::new()
        } else {
            let (clusters, not_ready) = talos::bring_up_clusters(
                lab_id,
                &lab_clusters,
                Duration::from_secs(ready_timeout_secs),
                &progress,
                &operation,
            )
            .await?;
            for name in not_ready {
                errors.push(data::UpError {
                    phase: "NodeReadiness".to_string(),
                    message: format!("Kubernetes cluster {name} did not become ready"),
                    is_critical: false,
                });
            }
            clusters
        };

        phases_completed.push("NodeReadiness".to_string());

        // ========================================================================
        // Build Response
        // ========================================================================

        let summary = data::UpSummary {
            containers_created: container_nodes.len(),
            vms_created: vm_nodes.len(),
            unikernels_created: unikernel_nodes.len(),
            networks_created: 1 + bridges_detailed.len(), // management + shared bridges
            bridges_created: links_detailed.len() * 2,    // 2 bridges per p2p link
            interfaces_created: lab_link_data.len() * 2,  // 2 veth interfaces per link
        };

        let critical_errors = errors.iter().filter(|e| e.is_critical).count();
        let warnings = errors.iter().filter(|e| !e.is_critical).count();
        let success = critical_errors == 0;
        let total_time = start_time.elapsed().as_secs();

        let response = data::UpResponse {
            tailscale: tailnet_status,
            success,
            lab_info: lab_info.clone(),
            total_time_secs: total_time,
            phases_completed: phases_completed.clone(),
            summary: summary.clone(),
            nodes: node_info_list,
            errors: errors.clone(),
            ssh_config: ssh_files.ssh_config,
            known_hosts: ssh_files.known_hosts,
            ssh_private_key,
            clusters,
        };

        tracing::info!(
            lab_id = %lab_id,
            lab_name = %manifest.name,
            success = success,
            total_time_secs = total_time,
            containers_created = summary.containers_created,
            vms_created = summary.vms_created,
            unikernels_created = summary.unikernels_created,
            networks_created = summary.networks_created,
            bridges_created = summary.bridges_created,
            interfaces_created = summary.interfaces_created,
            critical_errors = critical_errors,
            warnings = warnings,
            phases_completed = phases_completed.len(),
            "Lab creation completed"
        );

        Ok(response)
    }; // end resource_creation async block

    let op_attrs = &[KeyValue::new("operation.type", "up")];

    match resource_creation.await {
        Ok(response) => {
            state
                .metrics
                .operation_duration
                .record(start_time.elapsed().as_secs_f64(), op_attrs);
            Ok(response)
        }
        Err(e) => {
            state
                .metrics
                .operation_duration
                .record(start_time.elapsed().as_secs_f64(), op_attrs);

            match operation.stop_reason() {
                Some(StopReason::Destroy) => {
                    // The lab is being destroyed, which removes everything this run created.
                    tracing::info!(lab_id = %lab_id, "Lab creation stopped because the lab is being destroyed");
                    return Err(e);
                }
                Some(StopReason::Request) => {
                    // Stopped by the user: roll back what this run created.
                    tracing::info!(lab_id = %lab_id, "Lab creation stopped on request, starting auto-cleanup");
                    let _ = progress.send_status(
                        "Lab creation stopped, removing what it created...".to_string(),
                        StatusKind::Info,
                    );
                }
                None => {
                    state.metrics.error_count.add(1, op_attrs);
                    tracing::error!(
                        lab_id = %lab_id,
                        error = ?e,
                        "Lab creation failed after resource creation began, starting auto-cleanup"
                    );
                    let _ = progress.send_status(
                        "Lab creation failed, cleaning up resources...".to_string(),
                        StatusKind::Info,
                    );
                }
            }

            match clean::clean_lab_resources(lab_id, state).await {
                Ok(clean_response) => {
                    if clean_response.success {
                        tracing::info!(lab_id = %lab_id, "Auto-cleanup completed successfully");
                        let _ = progress.send_status(
                            "Cleanup completed successfully".to_string(),
                            StatusKind::Done,
                        );
                    } else {
                        tracing::warn!(
                            lab_id = %lab_id,
                            errors = ?clean_response.errors,
                            "Auto-cleanup completed with errors"
                        );
                        let _ = progress.send_status(
                            "Cleanup completed with some errors".to_string(),
                            StatusKind::Info,
                        );
                    }
                }
                Err(clean_err) => {
                    tracing::error!(
                        lab_id = %lab_id,
                        error = ?clean_err,
                        "Auto-cleanup failed"
                    );
                    let _ = progress
                        .send_status(format!("Cleanup failed: {clean_err}"), StatusKind::Info);
                }
            }

            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skipped_vm_is_ready_before_polling() {
        assert!(skips_readiness_before_polling(true, false, false));
    }

    #[test]
    fn test_talos_worker_is_ready_before_polling() {
        assert!(skips_readiness_before_polling(false, false, true));
    }

    #[test]
    fn test_skipped_container_waits_for_polling_to_start_it() {
        // Containers are started during polling, so skipping their ready check there would
        // skip starting them.
        assert!(!skips_readiness_before_polling(true, true, false));
    }

    #[test]
    fn test_node_without_skip_is_polled() {
        assert!(!skips_readiness_before_polling(false, false, false));
        assert!(!skips_readiness_before_polling(false, true, false));
    }

    #[test]
    fn omitted_clone_method_is_preserved_during_expansion() {
        let expanded = process_manifest_nodes(&[topology::Node {
            model: data::NodeModel::UbuntuLinux,
            ..Default::default()
        }]);
        assert!(
            serde_json::to_value(expanded[0].boot_disk_clone_method)
                .unwrap()
                .is_null()
        );
    }

    #[test]
    fn talos_nodes_default_to_a_boot_disk_large_enough_for_talos() {
        let expanded = process_manifest_nodes(&[
            topology::Node {
                name: "talos".into(),
                model: data::NodeModel::TalosLinux,
                ..Default::default()
            },
            topology::Node {
                name: "talos-big".into(),
                model: data::NodeModel::TalosLinux,
                boot_disk_size: Some(50),
                ..Default::default()
            },
            topology::Node {
                name: "ubuntu".into(),
                model: data::NodeModel::UbuntuLinux,
                ..Default::default()
            },
        ]);
        assert_eq!(expanded[0].boot_disk_size, Some(20));
        assert_eq!(expanded[1].boot_disk_size, Some(50));
        assert_eq!(expanded[2].boot_disk_size, None);
    }

    #[test]
    fn node_expansion_preserves_clone_method_options() {
        let nodes = vec![
            topology::Node {
                name: "default-vm".into(),
                model: data::NodeModel::UbuntuLinux,
                ..Default::default()
            },
            topology::Node {
                name: "copy-vm".into(),
                model: data::NodeModel::UbuntuLinux,
                boot_disk_clone_method: Some(data::DiskImageCloneMethod::Copy),
                ..Default::default()
            },
            topology::Node {
                name: "overlay-vm".into(),
                model: data::NodeModel::UbuntuLinux,
                boot_disk_clone_method: Some(data::DiskImageCloneMethod::Overlay),
                ..Default::default()
            },
            topology::Node {
                name: "container".into(),
                model: data::NodeModel::GenericContainer,
                ..Default::default()
            },
            topology::Node {
                name: "unikernel".into(),
                model: data::NodeModel::GenericUnikernel,
                ..Default::default()
            },
            topology::Node {
                name: "unsupported-overlay".into(),
                model: data::NodeModel::GenericContainer,
                boot_disk_clone_method: Some(data::DiskImageCloneMethod::Overlay),
                ..Default::default()
            },
        ];
        let expanded = process_manifest_nodes(&nodes);
        assert_eq!(expanded[0].boot_disk_clone_method, None);
        assert_eq!(
            expanded[1].boot_disk_clone_method,
            Some(data::DiskImageCloneMethod::Copy)
        );
        assert_eq!(
            expanded[2].boot_disk_clone_method,
            Some(data::DiskImageCloneMethod::Overlay)
        );
        assert_eq!(expanded[3].boot_disk_clone_method, None);
        assert_eq!(expanded[4].boot_disk_clone_method, None);
        // Preserve an explicit unsupported request so validation rejects it.
        assert_eq!(
            expanded[5].boot_disk_clone_method,
            Some(data::DiskImageCloneMethod::Overlay)
        );
    }
}
