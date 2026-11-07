//! Stage 1: loading the node and its lab, and every check that can refuse the redeploy.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::ops::RangeInclusive;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_host_access;
use crate::daemon::state::AppState;
use crate::services::guest_account::{self, GuestAccount};
use crate::services::image_lifecycle::{ImageLease, lease_image, validate_disk_method};
use crate::services::lab_operations::LabOperation;
use crate::services::node_resources;
use crate::services::progress::ProgressSender;
use crate::services::talos;
use crate::services::up::process_manifest_nodes;
use crate::tls::lab_ca::{self, LabCa};
use crate::validate::{reject_provisioners, validate_node_secrets};

use libvirt::QemuConnection;
use shared::data;
use shared::data::{DbLab, DbNode, RecordId, RedeployRequest, StatusKind};
use shared::konst::{LAB_CERTS_DIR, LAB_FILE_NAME, READINESS_TIMEOUT, SHERPA_LABS_PATH};
use shared::util;
use shared::validators::effective_data_interface_count;
use topology::find_talos_cluster;
use topology::validate::{
    check_client_read_files, check_performance_tuning, check_ptx_platform, check_watchdog,
};

/// The node being redeployed and its lab, loaded and checked before the node is torn down.
/// Every later stage takes it.
pub(super) struct Redeploy<'a> {
    pub(super) state: &'a AppState,
    pub(super) progress: &'a ProgressSender,
    pub(super) operation: &'a LabOperation,
    pub(super) lab_id: String,
    pub(super) node_name: String,
    pub(super) lab_dir: String,
    pub(super) lab_record_id: RecordId,
    /// The lab info, with the node's SSH username as it will be after the redeploy.
    pub(super) lab_info: data::LabInfo,
    pub(super) mgmt_net: data::SherpaNetwork,
    pub(super) dns: data::Dns,
    pub(super) certs_dir: String,
    pub(super) lab_ca: LabCa,
    pub(super) sherpa_user: data::User,
    pub(super) qemu_conn: Arc<QemuConnection>,
    /// The node's record; its resources are reserved again when it is rebuilt.
    pub(super) db_node: DbNode,
    pub(super) node_record_id: RecordId,
    pub(super) node_image: data::NodeConfig,
    /// The node as the manifest describes it now.
    pub(super) target_node: topology::NodeExpanded,
    pub(super) data_interface_count: u8,
    pub(super) ipv4: Ipv4Addr,
    pub(super) ipv6: Option<Ipv6Addr>,
    /// The lab owner's account, when the node takes it.
    pub(super) owner_account: Option<GuestAccount>,
    pub(super) owner_username: String,
    /// Secret files for a container node; `None` keeps its current ones.
    pub(super) secrets: Option<Vec<data::SecretFile>>,
    pub(super) ready_timeout: Duration,
    /// Keeps the node's image until its disks are cloned.
    pub(super) image_guard: ImageLease,
}

/// Load the node, its image and its lab, and run every check that can refuse the redeploy
/// (stage 1). Nothing is changed, so a refused redeploy leaves the node as it was.
#[instrument(skip_all, fields(lab_id = %request.lab_id, node_name = %request.node_name), level = "debug")]
pub(super) async fn load_redeploy<'a>(
    ctx: &AuthContext,
    request: RedeployRequest,
    db_lab: &DbLab,
    state: &'a AppState,
    progress: &'a ProgressSender,
    operation: &'a LabOperation,
) -> Result<Redeploy<'a>> {
    let RedeployRequest {
        lab_id,
        node_name,
        manifest,
        secrets,
    } = request;
    let _ = progress.send_status(
        format!("Loading context for node: {}", node_name),
        StatusKind::Progress,
    );

    let manifest: topology::Manifest =
        serde_json::from_value(manifest).context("Failed to deserialize manifest")?;
    require_host_access(ctx, &manifest.nodes)?;
    check_client_read_files(&manifest.nodes).context("Manifest validation failed: files")?;
    reject_provisioners(&manifest).context("Manifest validation failed: provisioners")?;
    if let Some(files) = secrets.as_deref()
        && !files.is_empty()
    {
        validate_node_secrets(
            &manifest,
            &BTreeMap::from([(node_name.clone(), files.to_vec())]),
        )
        .context("Manifest validation failed: container secret files")?;
    }

    let mut target_node = manifest_node(&manifest, &node_name)?;
    let talos_membership = find_talos_cluster(
        manifest.talos_clusters.as_deref().unwrap_or_default(),
        &node_name,
    );
    talos::ensure_redeployable(&node_name, talos_membership)?;

    let db = &state.db;
    let lab_record_id = db::get_lab_id(db_lab).context("Failed to get lab record ID")?;
    let db_node = db::get_node_by_name_and_lab(db, &node_name, lab_record_id.clone())
        .await
        .context(format!(
            "Node '{}' not found in database for lab '{}'",
            node_name, lab_id
        ))?;
    let node_record_id = db::get_node_id(&db_node).context("Failed to get node record ID")?;

    let node_images = db::list_node_images_by_ids(db, vec![db_node.image.clone()])
        .await
        .context("Failed to get node image config")?;
    let node_image = node_images
        .first()
        .ok_or_else(|| anyhow!("Node image config not found for node '{}'", node_name))?
        .clone();
    ensure!(
        target_node.model == node_image.model,
        "Redeploy cannot change node model; expected {}",
        node_image.model
    );
    ensure!(
        !matches!(node_image.kind, data::NodeKind::Unikernel),
        "Redeploy not supported for unikernel nodes"
    );
    let data_interface_count =
        effective_data_interface_count(&node_name, target_node.data_interface_count, &node_image)
            .context(format!(
            "Data interface count validation failed for node: {}",
            node_name
        ))?;

    let image_guard = lease_image(node_image.model, &node_image.version).await;
    target_node.version = Some(node_image.version.clone());
    validate_disk_method(
        target_node.boot_disk_clone_method,
        &node_image,
        &state.config.images_dir,
        target_node.boot_disk_size,
    )?;

    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let lab_file = util::load_file(&format!("{lab_dir}/{LAB_FILE_NAME}"))
        .context("Unable to load lab file")?;
    let mut lab_info: data::LabInfo = lab_file.parse().context("Failed to parse lab info file")?;
    let mgmt_net = lab_management_network(&lab_info);
    let dns = if let Some(ref v6) = mgmt_net.v6 {
        util::default_dns_dual_stack(&lab_info.ipv4_network, &v6.prefix)?
    } else {
        util::default_dns(&lab_info.ipv4_network)?
    };
    let certs_dir = format!("{lab_dir}/{LAB_CERTS_DIR}");
    let lab_ca = lab_ca::load_lab_ca(Path::new(&certs_dir), &lab_id)?;
    // A cluster worker rejoins with a config built from the cluster's saved secrets.
    if let Some((talos_cluster, _)) = talos_membership {
        talos::secrets::ensure_supported_version(&node_name, &node_image.version)?;
        let cluster = talos::load_cluster(&lab_id, &talos_cluster.name)?;
        target_node.ztp_config = Some(talos::node_machine_config(
            &cluster,
            &node_name,
            &mgmt_net.v4,
            &lab_ca.pair.cert,
        )?);
    }
    let sherpa_user = util::sherpa_user().context("Failed to get sherpa user")?;
    let (ipv4, ipv6) = node_management_addresses(&db_node, &mgmt_net)?;
    target_node.ipv6_address = ipv6;

    let owner_account = if manifest.provision_current_user
        && guest_account::provisions_owner(&target_node, &node_image)
    {
        let account = guest_account::resolve_owner(db, db_lab.user.clone()).await?;
        if account.is_none() {
            let _ = progress.send_status(
                    format!(
                        "Username '{}' cannot be a login on lab nodes; '{node_name}' uses the sherpa account",
                        ctx.username
                    ),
                    StatusKind::Info,
                );
        }
        account
    } else {
        None
    };
    // The node must take one of the owner's keys before teardown.
    let node_login = match &owner_account {
        Some(account) => {
            let node_account = guest_account::node_account(account, &node_name, &node_image)?;
            if !node_account.skipped_keys.is_empty() {
                let _ = progress.send_status(
                    format!(
                        "Node '{node_name}' does not take these SSH keys of '{}': {}",
                        account.username,
                        node_account.skipped_keys.join(", ")
                    ),
                    StatusKind::Info,
                );
            }
            Some(node_account.account.username)
        }
        None => None,
    };
    // Calculate the next SSH config before teardown, preserving every other node's identity.
    lab_info.set_ssh_username(&node_name, node_login.as_deref());
    let _ = progress.send_status(
        format!("Context loaded for node: {}", node_name),
        StatusKind::Done,
    );

    let qemu_conn = Arc::new(
        state
            .qemu
            .connect()
            .context("Failed to connect to libvirt")?,
    );

    // The node's performance tuning must still fit the server before the node is destroyed.
    check_performance_tuning(&target_node, &node_image).context(format!(
        "Performance tuning validation failed for node: {node_name}"
    ))?;
    check_watchdog(&target_node, &node_image)
        .context(format!("Watchdog validation failed for node: {node_name}"))?;
    check_ptx_platform(&target_node, &node_image)
        .context(format!("Platform validation failed for node: {node_name}"))?;
    node_resources::check_cpu_pins_on_server(
        db,
        &qemu_conn,
        &[&target_node],
        Some(&node_record_id),
    )
    .await
    .context("CPU pinning validation failed")?;

    Ok(Redeploy {
        state,
        progress,
        operation,
        lab_dir,
        lab_record_id,
        lab_info,
        mgmt_net,
        dns,
        certs_dir,
        lab_ca,
        sherpa_user,
        qemu_conn,
        db_node,
        node_record_id,
        node_image,
        target_node,
        data_interface_count,
        ipv4,
        ipv6,
        owner_account,
        owner_username: ctx.username.clone(),
        secrets,
        ready_timeout: Duration::from_secs(manifest.ready_timeout.unwrap_or(READINESS_TIMEOUT)),
        image_guard,
        lab_id,
        node_name,
    })
}

/// The node in the manifest, with its custom ZTP config decoded.
fn manifest_node(manifest: &topology::Manifest, node_name: &str) -> Result<topology::NodeExpanded> {
    let mut node = process_manifest_nodes(&manifest.nodes)
        .into_iter()
        .find(|n| n.name == node_name)
        .ok_or_else(|| anyhow!("Node '{}' not found in manifest", node_name))?;
    if let Some(ref encoded) = node.ztp_config {
        let decoded = util::base64_decode(encoded)
            .with_context(|| format!("Failed to decode ztp_config for node '{}'", node_name))?;
        node.ztp_config = Some(decoded);
    }
    Ok(node)
}

/// The lab's management network, from its lab info. IPv6 only when the lab has it.
fn lab_management_network(lab_info: &data::LabInfo) -> data::SherpaNetwork {
    data::SherpaNetwork {
        v4: data::NetworkV4 {
            prefix: lab_info.ipv4_network,
            first: lab_info.ipv4_gateway,
            last: lab_info.ipv4_network.broadcast(),
            boot_server: lab_info.ipv4_router,
            network: lab_info.ipv4_network.network(),
            subnet_mask: lab_info.ipv4_network.netmask(),
            hostmask: lab_info.ipv4_network.hostmask(),
            prefix_length: lab_info.ipv4_network.prefix_len(),
        },
        v6: match (
            lab_info.ipv6_network,
            lab_info.ipv6_gateway,
            lab_info.ipv6_router,
        ) {
            (Some(v6_net), Some(v6_gw), Some(v6_rtr)) => Some(data::NetworkV6 {
                prefix: v6_net,
                first: v6_gw,
                last: util::get_ipv6_addr(&v6_net, u32::MAX).unwrap_or(v6_net.network()),
                boot_server: v6_rtr,
                network: v6_net.network(),
                prefix_length: v6_net.prefix_len(),
            }),
            _ => None,
        },
    }
}

/// The node's management addresses. Saved addresses win, so a node keeps the address the
/// lab's dnsmasq and SSH config already hold, even if the address layout has changed since
/// the lab was deployed. Nodes without saved addresses get them from their index.
fn node_management_addresses(
    db_node: &DbNode,
    mgmt_net: &data::SherpaNetwork,
) -> Result<(Ipv4Addr, Option<Ipv6Addr>)> {
    let ipv4 = match db_node.mgmt_ipv4.as_deref() {
        Some(saved) => saved
            .parse()
            .with_context(|| format!("Invalid saved management address: {saved}"))?,
        None => util::get_ipv4_addr(
            &mgmt_net.v4.prefix,
            util::node_management_host_offset(db_node.index)?,
        )?,
    };
    let ipv6 = match (db_node.mgmt_ipv6.as_deref(), mgmt_net.v6.as_ref()) {
        (_, None) => None,
        (Some(saved), Some(_)) => Some(
            saved
                .parse()
                .with_context(|| format!("Invalid saved IPv6 management address: {saved}"))?,
        ),
        (None, Some(v6)) => Some(util::get_ipv6_addr(
            &v6.prefix,
            util::node_management_host_offset(db_node.index)?,
        )?),
    };
    Ok((ipv4, ipv6))
}

/// The indices of the node's data interfaces, after its management interface (0) and its
/// reserved interfaces.
pub(super) fn data_interface_indices(
    node_image: &data::NodeConfig,
    data_interface_count: u8,
    node_name: &str,
) -> Result<RangeInclusive<u8>> {
    let first = node_image
        .reserved_interface_count
        .checked_add(1)
        .ok_or_else(|| anyhow!("Reserved interface count overflow for node {}", node_name))?;
    let last = node_image
        .reserved_interface_count
        .checked_add(data_interface_count)
        .ok_or_else(|| anyhow!("Data interface count overflow for node {}", node_name))?;
    Ok(first..=last)
}

#[cfg(test)]
mod tests {
    use super::*;

    use shared::data::NodeState;

    fn mgmt_net(with_v6: bool) -> data::SherpaNetwork {
        let prefix = util::get_ipv4_network("172.31.1.0/24").unwrap();
        let v6_prefix = util::get_ipv6_network("fd00:b00b:0:1::/64").unwrap();
        let v6 = if with_v6 {
            Some(data::NetworkV6 {
                prefix: v6_prefix,
                first: "fd00:b00b:0:1::1".parse().unwrap(),
                last: "fd00:b00b:0:1:ffff:ffff:ffff:ffff".parse().unwrap(),
                boot_server: "fd00:b00b:0:1::2".parse().unwrap(),
                network: v6_prefix.network(),
                prefix_length: v6_prefix.prefix_len(),
            })
        } else {
            None
        };
        data::SherpaNetwork {
            v4: data::NetworkV4 {
                prefix,
                first: "172.31.1.1".parse().unwrap(),
                last: prefix.broadcast(),
                boot_server: "172.31.1.2".parse().unwrap(),
                network: prefix.network(),
                subnet_mask: prefix.netmask(),
                hostmask: prefix.hostmask(),
                prefix_length: prefix.prefix_len(),
            },
            v6,
        }
    }

    fn db_node(index: u16, mgmt_ipv4: Option<&str>, mgmt_ipv6: Option<&str>) -> DbNode {
        DbNode {
            id: None,
            name: "node1".to_string(),
            image: RecordId::new("node_image", "img"),
            index,
            lab: RecordId::new("lab", "lab"),
            mgmt_ipv4: mgmt_ipv4.map(str::to_string),
            mgmt_ipv6: mgmt_ipv6.map(str::to_string),
            mgmt_mac: None,
            state: NodeState::Running,
            cpu_count: None,
            memory: None,
            vnc_port: None,
            ssh_host_key: None,
            cpu_pinning: None,
            qemu_thread_cpus: None,
        }
    }

    #[test]
    fn test_saved_addresses_survive_a_layout_change() {
        // A node deployed under the old layout (node 1 at .11) keeps its address.
        let node = db_node(1, Some("172.31.1.11"), Some("fd00:b00b:0:1::b"));
        let (ipv4, ipv6) = node_management_addresses(&node, &mgmt_net(true)).unwrap();
        assert_eq!(ipv4.to_string(), "172.31.1.11");
        assert_eq!(ipv6.unwrap().to_string(), "fd00:b00b:0:1::b");
    }

    #[test]
    fn test_unsaved_addresses_come_from_the_node_block() {
        let node = db_node(1, None, None);
        let (ipv4, ipv6) = node_management_addresses(&node, &mgmt_net(true)).unwrap();
        assert_eq!(ipv4.to_string(), "172.31.1.21");
        assert_eq!(ipv6.unwrap().to_string(), "fd00:b00b:0:1::15");
    }

    #[test]
    fn test_no_ipv6_address_without_an_ipv6_network() {
        let node = db_node(1, Some("172.31.1.21"), Some("fd00:b00b:0:1::15"));
        let (_, ipv6) = node_management_addresses(&node, &mgmt_net(false)).unwrap();
        assert!(ipv6.is_none());
    }

    #[test]
    fn test_invalid_saved_address_is_an_error() {
        let node = db_node(1, Some("not-an-ip"), None);
        assert!(node_management_addresses(&node, &mgmt_net(false)).is_err());
    }

    #[test]
    fn data_interfaces_follow_the_reserved_ones() {
        let mut image = data::NodeConfig::ubuntu_linux();
        image.reserved_interface_count = 2;
        assert_eq!(data_interface_indices(&image, 3, "n1").unwrap(), 3..=5);
    }

    #[test]
    fn a_node_without_data_interfaces_has_none() {
        let mut image = data::NodeConfig::ubuntu_linux();
        image.reserved_interface_count = 0;
        assert!(data_interface_indices(&image, 0, "n1").unwrap().is_empty());
    }

    #[test]
    fn an_interface_count_past_the_index_range_is_an_error() {
        let mut image = data::NodeConfig::ubuntu_linux();
        image.reserved_interface_count = u8::MAX;
        assert!(data_interface_indices(&image, 1, "n1").is_err());
    }
}
