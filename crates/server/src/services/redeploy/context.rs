//! Stage 1: loading the node and its lab, and every check that can refuse the redeploy.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_host_access;
use crate::daemon::state::AppState;
use crate::konst::LAB_CERTS_DIR;
use crate::services::admission::{self, Admission};
use crate::services::error::ServiceError;
use crate::services::guest_account::{self, GuestAccount};
use crate::services::image_lifecycle::{
    ImageLease, check_container_image, lease_image, validate_disk_method,
};
use crate::services::lab_definition;
use crate::services::lab_info;
use crate::services::lab_operations::LabOperation;
use crate::services::progress::ProgressSender;
use crate::services::talos;
use crate::services::up::process_manifest_nodes;
use crate::tls::lab_ca::{self, LabCa};
use crate::validate::{
    reject_provisioners, validate_container_repo, validate_node_secrets, validate_node_ztp,
};

use libvirt::QemuConnection;
use shared::data;
use shared::data::{DbLab, DbNode, RecordId, RedeployRequest, StatusKind};
use shared::konst::{READINESS_TIMEOUT, SHERPA_LABS_PATH};
use shared::util;
use shared::validators::effective_data_interface_count;
use topology::validate::{
    check_bridges, check_client_read_files, check_duplicate_device, check_lab_name,
    check_links_to_itself, check_node_file_settings, check_node_kind_settings,
    check_nonzero_node_settings, check_ptx_platform, check_ready_timeout, check_text_file_modes,
    check_vm_tuning, check_windows_startup_script_names, validate_environment_variables,
};
use topology::{TopologyError, find_talos_cluster};

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
    pub(super) lab_info: data::LabInfo,
    pub(super) mgmt_net: data::SherpaNetwork,
    pub(super) dns: data::Dns,
    pub(super) certs_dir: String,
    pub(super) lab_ca: LabCa,
    pub(super) qemu_conn: Arc<QemuConnection>,
    pub(super) db_node: DbNode,
    pub(super) node_record_id: RecordId,
    pub(super) node_image: data::NodeConfig,
    pub(super) target_node: topology::NodeExpanded,
    pub(super) definition: RedeployDefinition,
    pub(super) data_interface_count: u8,
    pub(super) ipv4: Ipv4Addr,
    pub(super) ipv6: Option<Ipv6Addr>,
    pub(super) owner_account: GuestAccount,
    pub(super) secrets: Option<Vec<data::SecretFile>>,
    pub(super) image_guard: ImageLease,
    pub(super) admission: Admission,
}

/// The manifest a node is redeployed from, as the client sent it, and its definition.
pub(super) struct RedeployDefinition {
    pub(super) manifest: topology::Manifest,
    pub(super) as_written: Option<topology::Manifest>,
}

impl Redeploy<'_> {
    /// How long the node has to become ready: the manifest's `ready_timeout`.
    pub(super) fn ready_timeout(&self) -> Duration {
        Duration::from_secs(
            self.definition
                .manifest
                .ready_timeout
                .unwrap_or(READINESS_TIMEOUT),
        )
    }
}

/// The node's record and image, and its data interface count, as the redeploy finds them.
struct LoadedNode {
    db_node: DbNode,
    node_record_id: RecordId,
    node_image: data::NodeConfig,
    data_interface_count: u8,
    image_guard: ImageLease,
}

/// The lab's directory, details, management network, DNS and CA.
struct LabFiles {
    lab_dir: String,
    lab_info: data::LabInfo,
    mgmt_net: data::SherpaNetwork,
    dns: data::Dns,
    certs_dir: String,
    lab_ca: LabCa,
}

/// Load the node, its image and its lab, and run every check that can refuse the redeploy
/// (stage 1). A refused redeploy changes nothing, and leaves the node as it was; the last
/// check, admission, claims the node's new pins beside its old ones.
#[instrument(skip_all, fields(lab_id = %request.lab_id, node_name = %request.node_name), level = "debug")]
pub(super) async fn load_redeploy<'a>(
    ctx: &AuthContext,
    request: RedeployRequest,
    db_lab: &DbLab,
    owner_account: GuestAccount,
    state: &'a AppState,
    progress: &'a ProgressSender,
    operation: &'a LabOperation,
) -> Result<Redeploy<'a>> {
    let _ = progress.send_status(
        format!("Loading context for node: {}", request.node_name),
        StatusKind::Progress,
    );

    let definition = check_manifest(ctx, &request)?;
    let RedeployRequest {
        lab_id,
        node_name,
        secrets,
        ..
    } = request;
    let manifest = &definition.manifest;
    let mut target_node = manifest_node(manifest, &node_name)?;

    let lab_record_id = db::get_lab_id(db_lab).context("Failed to get lab record ID")?;
    let talos_clusters =
        stored_talos_clusters(&state.db, &lab_record_id, manifest, &node_name).await?;
    let talos_membership = find_talos_cluster(&talos_clusters, &node_name);
    talos::ensure_redeployable(&node_name, talos_membership)?;
    let LoadedNode {
        db_node,
        node_record_id,
        node_image,
        data_interface_count,
        image_guard,
    } = load_node(
        state,
        &lab_record_id,
        &lab_id,
        &mut target_node,
        &talos_clusters,
    )
    .await?;
    target_node.version = Some(node_image.version.clone());
    let manifest_entry = manifest
        .nodes
        .iter()
        .find(|node| node.name == node_name)
        .ok_or_else(|| anyhow!("Node '{node_name}' not found in manifest"))?;
    check_node_file_settings(manifest_entry, &node_image).context(format!(
        "Node settings validation failed for node: {node_name}"
    ))?;

    let LabFiles {
        lab_dir,
        mut lab_info,
        mgmt_net,
        dns,
        certs_dir,
        lab_ca,
    } = load_lab_files(&state.db, db_lab).await?;
    // A cluster worker rejoins with a config built from the cluster's saved secrets.
    if talos_membership.is_some() {
        let config =
            worker_rejoin_config(state, db_lab, &target_node, &node_image, &mgmt_net, &lab_ca);
        target_node.ztp.config = Some(config.await?);
    }
    let (ipv4, ipv6) = node_management_addresses(&db_node, &mgmt_net)?;
    target_node.ipv6_address = ipv6;

    // Calculate the next SSH config before teardown, preserving every other node's identity.
    let node_login = node_login(&owner_account, &target_node, &node_image, progress)?;
    lab_info.set_ssh_username(&node_name, node_login.as_deref());
    let _ = progress.send_status(
        format!("Context loaded for node: {node_name}"),
        StatusKind::Done,
    );

    let (qemu_conn, admitted) =
        check_fits_server(state, ctx, progress, &lab_id, &target_node, &node_image).await?;

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
        qemu_conn,
        db_node,
        node_record_id,
        node_image,
        target_node,
        data_interface_count,
        ipv4,
        ipv6,
        owner_account,
        secrets,
        definition,
        image_guard,
        admission: admitted,
        lab_id,
        node_name,
    })
}

/// The request's manifest, with the checks a redeploy shares with `sherpa up`: the lab name,
/// host access, duplicate node names, the ready timeout, the node settings that cannot be 0, a
/// link to itself, the bridges' names and links, environment variable keys, text file modes,
/// Windows startup script names, the files the client read, provisioners, and the node's secret
/// files. With the parts as written the client sent beside it, the node's definition is built
/// and its provisioners checked.
fn check_manifest(ctx: &AuthContext, request: &RedeployRequest) -> Result<RedeployDefinition> {
    let node_name = request.node_name.as_str();
    let manifest =
        lab_definition::parse_manifest(&request.manifest, request.file_contents.as_ref())?;
    check_lab_name(&manifest.name).map_err(|error| manifest_refusal("lab name", error))?;
    require_host_access(ctx, &manifest.nodes)?;
    check_duplicate_device(&manifest.nodes)
        .map_err(|error| manifest_refusal("duplicate nodes", error))?;
    check_ready_timeout(manifest.ready_timeout)
        .map_err(|error| manifest_refusal("ready timeout", error))?;
    check_nonzero_node_settings(&manifest.nodes)
        .map_err(|error| manifest_refusal("node settings", error))?;
    check_links_to_itself(manifest.links.as_deref().unwrap_or_default())
        .map_err(|error| manifest_refusal("links", error))?;
    check_bridges(manifest.bridges.as_deref().unwrap_or_default())
        .map_err(|error| manifest_refusal("bridges", error))?;
    validate_environment_variables(&manifest.nodes)
        .map_err(|error| manifest_refusal("environment variables", error))?;
    check_text_file_modes(&manifest.nodes)
        .map_err(|error| manifest_refusal("text files", error))?;
    check_windows_startup_script_names(&manifest.nodes)
        .context("Manifest validation failed: startup scripts")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    check_client_read_files(&manifest.nodes)
        .context("Manifest validation failed: files")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    reject_provisioners(&manifest)
        .context("Manifest validation failed: provisioners")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    let as_written = match lab_definition::parse_as_written(request.as_written.as_ref())? {
        Some(parts) => Some(
            lab_definition::definition(&manifest, Some(&parts))
                .context("Manifest validation failed: the manifest as written")
                .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?,
        ),
        None => None,
    };
    if let Some(files) = request.secrets.as_deref()
        && !files.is_empty()
    {
        validate_node_secrets(
            &manifest,
            &BTreeMap::from([(node_name.to_owned(), files.to_vec())]),
        )
        .context("Manifest validation failed: container secret files")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    }
    Ok(RedeployDefinition {
        manifest,
        as_written,
    })
}

/// The refusal of a manifest that fails `check`, carrying the check's `TopologyError` as its
/// source; a failure of any other type is refused with its chain as the message.
fn manifest_refusal(check: &'static str, error: anyhow::Error) -> anyhow::Error {
    match error.downcast::<TopologyError>() {
        Ok(refusal) => ServiceError::ManifestCheckFailed { check, refusal }.into(),
        Err(error) => {
            ServiceError::Invalid(format!("Manifest validation failed: {check}: {error:#}")).into()
        }
    }
}

/// The Talos clusters of the lab `lab_record_id`'s stored definition, which node `node_name` is
/// rebuilt with, once `manifest` is found to give the node the cluster and role the definition
/// does. A lab with no stored definition has no stored cluster to rebuild a member of.
#[instrument(skip(db, lab_record_id, manifest), level = "debug")]
async fn stored_talos_clusters(
    db: &db::Database,
    lab_record_id: &RecordId,
    manifest: &topology::Manifest,
    node_name: &str,
) -> Result<Vec<topology::TalosCluster>> {
    let stored = db::read_lab_definition(db, lab_record_id)
        .await
        .context("Failed to read the lab's stored definition")?
        .and_then(|definition| definition.talos_clusters)
        .unwrap_or_default();
    talos::ensure_same_membership(
        node_name,
        find_talos_cluster(&stored, node_name),
        find_talos_cluster(
            manifest.talos_clusters.as_deref().unwrap_or_default(),
            node_name,
        ),
    )?;
    Ok(stored)
}

/// The node's record and the image it is rebuilt from, leased, with the node's ZTP setting and
/// disk method checked against it, and a container's image resolved and checked as `up` does:
/// the target node's `image` is set to the repo it is created from. The image is found as `up`
/// finds it, by the node's model, kind and manifest `version`, or else their default, so a
/// node whose manifest names another version is rebuilt from that version. The model and kind
/// cannot change, and unikernels cannot be redeployed.
#[instrument(skip_all, fields(%lab_id, node_name = %target_node.name), level = "debug")]
async fn load_node(
    state: &AppState,
    lab_record_id: &RecordId,
    lab_id: &str,
    target_node: &mut topology::NodeExpanded,
    talos_clusters: &[topology::TalosCluster],
) -> Result<LoadedNode> {
    let db = &state.db;
    let node_name = target_node.name.as_str();
    let db_node = db::get_node_by_name_and_lab(db, node_name, lab_record_id.clone())
        .await
        .map_err(|error| {
            // The lookup returns one error for a missing node and a failed query.
            tracing::warn!(error = %format!("{error:#}"), "Node lookup failed");
            ServiceError::NotFound(format!(
                "Node '{node_name}' not found in database for lab '{lab_id}'"
            ))
        })?;
    let node_record_id = db::get_node_id(&db_node).context("Failed to get node record ID")?;

    let current_images = db::list_node_images_by_ids(db, vec![db_node.image.clone()])
        .await
        .context("Failed to get node image config")?;
    let current_image = current_images
        .first()
        .ok_or_else(|| anyhow!("Node image config not found for node '{node_name}'"))?;
    if target_node.model_kind.model() != current_image.model
        || target_node.model_kind.kind() != current_image.kind()
    {
        return Err(ServiceError::Invalid(format!(
            "Redeploy cannot change node model or kind; expected {} ({})",
            current_image.model,
            current_image.kind()
        ))
        .into());
    }
    let node_image =
        redeploy_image(db, target_node.model_kind, target_node.version.as_deref()).await?;
    if matches!(node_image.kind(), data::NodeKind::Unikernel) {
        return Err(
            ServiceError::Invalid("Redeploy not supported for unikernel nodes".to_owned()).into(),
        );
    }
    validate_node_ztp(target_node, &node_image, talos_clusters)?;
    let data_interface_count =
        effective_data_interface_count(node_name, target_node.interfaces.data_count, &node_image)
            .context(format!(
                "Data interface count validation failed for node: {node_name}"
            ))
            .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;

    let image_guard = lease_image(node_image.model_kind()?, &node_image.version).await;
    if node_image.kind() == data::NodeKind::Container {
        let repo = validate_container_repo(
            node_name,
            node_image.model,
            &node_image.version,
            node_image.container_repo(target_node.image.as_deref()),
        )?;
        check_container_image(
            &state.docker,
            node_name,
            node_image.model,
            &repo,
            &node_image.version,
        )
        .await?;
        target_node.image = Some(repo);
    }
    validate_disk_method(
        node_name,
        target_node.boot_disk.clone_method,
        &node_image,
        &state.config.images_dir,
        target_node.boot_disk.size,
    )?;
    Ok(LoadedNode {
        db_node,
        node_record_id,
        node_image,
        data_interface_count,
        image_guard,
    })
}

/// The image a redeployed node of `model_kind` is rebuilt from: its `version`, or else the
/// default of its model and kind.
///
/// # Errors
///
/// Fails when there is no such image, naming the model, kind and version, or the database
/// cannot be read.
#[instrument(skip(db), level = "debug")]
async fn redeploy_image(
    db: &db::Database,
    model_kind: data::ModelKind,
    version: Option<&str>,
) -> Result<data::NodeConfig> {
    let (model, kind) = (model_kind.model(), model_kind.kind());
    match version {
        Some(version) => db::find_node_image_by_model_kind_version(db, &model, &kind, version)
            .await
            .context("Failed to look up the node's image")?
            .ok_or_else(|| {
                ServiceError::ImageVersionNotFound {
                    model_kind,
                    version: version.to_owned(),
                }
                .into()
            }),
        None => db::find_default_node_image(db, &model, &kind)
            .await
            .context("Failed to look up the node's image")?
            .ok_or_else(|| ServiceError::DefaultImageNotFound { model_kind }.into()),
    }
}

/// The lab's details from the database, its management network and DNS from them, and its
/// CA from the lab's folder.
#[instrument(skip(db, db_lab), fields(lab_id = %db_lab.lab_id), level = "debug")]
async fn load_lab_files(db: &db::Database, db_lab: &DbLab) -> Result<LabFiles> {
    let lab_id = db_lab.lab_id.as_str();
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let lab_info = lab_info::load_lab_info(db, db_lab).await?;
    let mgmt_net = lab_management_network(&lab_info);
    let dns = util::default_dns(&mgmt_net);
    let certs_dir = format!("{lab_dir}/{LAB_CERTS_DIR}");
    let lab_ca = lab_ca::load_lab_ca(Path::new(&certs_dir), lab_id)?;
    Ok(LabFiles {
        lab_dir,
        lab_info,
        mgmt_net,
        dns,
        certs_dir,
        lab_ca,
    })
}

/// The machine config a Talos cluster worker rejoins with, built from the stored plan and
/// saved secrets of the cluster it is a member of.
async fn worker_rejoin_config(
    state: &AppState,
    db_lab: &DbLab,
    target_node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    mgmt_net: &data::SherpaNetwork,
    lab_ca: &LabCa,
) -> Result<String> {
    talos::secrets::ensure_supported_version(&target_node.name, &node_image.version)?;
    let clusters = talos::load_clusters(
        &state.db,
        Path::new(SHERPA_LABS_PATH),
        &db::get_lab_id(db_lab)?,
        &db_lab.lab_id,
    )
    .await?;
    let (cluster, _, _) =
        talos::find_member_cluster(&clusters, &target_node.name).ok_or_else(|| {
            ServiceError::Conflict(format!(
                "Node '{}' is not a member of a stored Talos cluster",
                target_node.name
            ))
        })?;
    talos::node_machine_config(cluster, &target_node.name, &mgmt_net.v4, &lab_ca.pair.cert)
}

/// The node's login: the owner's name when the node gets their account, which must take the
/// name and one of their keys before teardown, with keys it does not take reported; `admin` on
/// Cisco `FTDv`; else `None`, and the SSH config names the owner.
fn node_login(
    owner_account: &GuestAccount,
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    progress: &ProgressSender,
) -> Result<Option<String>> {
    if !guest_account::provisions_owner(node, node_image) {
        return Ok(guest_account::fixed_login(node_image.model).map(str::to_owned));
    }
    let node_account = guest_account::node_account(owner_account, &node.name, node_image)?;
    if !node_account.skipped_keys.is_empty() {
        let _ = progress.send_status(
            format!(
                "Node '{}' does not take these SSH keys of '{}': {}",
                node.name,
                owner_account.username,
                node_account.skipped_keys.join(", ")
            ),
            StatusKind::Info,
        );
    }
    Ok(Some(node_account.account.username))
}

/// The node's settings by kind, VM tuning, platform, CPU pins and memory must still fit the
/// server before the node is destroyed. The node being replaced does not count against itself.
/// Returns the libvirt connection the redeploy uses, and the node's admission, which claimed
/// its new pins beside its old ones. A warning for each overlap the node's stored pins already
/// had goes to the user.
#[instrument(skip_all, fields(node_name = %target_node.name), level = "debug")]
async fn check_fits_server(
    state: &AppState,
    requester: &AuthContext,
    progress: &ProgressSender,
    lab_id: &str,
    target_node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
) -> Result<(Arc<QemuConnection>, Admission)> {
    let node_name = target_node.name.as_str();
    check_node_kind_settings(target_node)
        .context(format!(
            "Node settings validation failed for node: {node_name}"
        ))
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    check_vm_tuning(target_node, node_image)
        .context(format!("VM tuning validation failed for node: {node_name}"))
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    check_ptx_platform(target_node, node_image)
        .context(format!("Platform validation failed for node: {node_name}"))
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    let qemu_conn = Arc::new(
        state
            .qemu
            .connect()
            .context("Failed to connect to libvirt")?,
    );
    let admitted = admission::admit_lab_nodes(
        &state.db,
        &qemu_conn,
        &state.docker,
        requester,
        state.config.host_memory,
        lab_id,
        &[(target_node, node_image)],
    )
    .await?;
    for warning in admitted.shared_cpus() {
        let _ = progress.send_status(format!("Warning: {warning}"), StatusKind::Info);
    }
    Ok((qemu_conn, admitted))
}

/// The node in the manifest, with its custom ZTP config decoded.
fn manifest_node(manifest: &topology::Manifest, node_name: &str) -> Result<topology::NodeExpanded> {
    let mut node = process_manifest_nodes(&manifest.nodes)?
        .into_iter()
        .find(|n| n.name == node_name)
        .ok_or_else(|| {
            ServiceError::NotFound(format!("Node '{node_name}' not found in manifest"))
        })?;
    if let Some(ref encoded) = node.ztp.config {
        let decoded = util::base64_decode(encoded)
            .with_context(|| format!("Failed to decode ztp.config for node '{node_name}'"))
            .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
        node.ztp.config = Some(decoded);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::request_parse::InvalidRequest;
    use db::DefaultRequest;
    use shared::data::{ModelKind, ReadyCheck};

    use shared::data::{NodeModel, NodeState};

    use shared::SharedError;

    use crate::services::error::ServiceCategory;
    use crate::services::guest_account::tests::test_account;
    use crate::services::lab_definition::tests::{lab_with, store};
    use tokio::sync::mpsc;

    const OWNER_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh";

    /// The login a redeploy records for a `model` node named `name`, owned by `owner`.
    fn login_after_redeploy(owner: &str, model: NodeModel, custom: bool) -> Result<Option<String>> {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut node = topology::NodeExpanded {
            name: "n1".to_owned(),
            model_kind: ModelKind::default_for(model),
            ..Default::default()
        };
        if custom {
            node.ztp.config = Some("hostname n1\n".to_owned());
        }
        node_login(
            &test_account(owner, &[OWNER_KEY]),
            &node,
            &data::ModelKind::default_for(model).node_config(),
            &ProgressSender::new(tx),
        )
    }

    #[test]
    fn api_errors_015_a_redeploy_whose_manifest_does_not_parse_names_the_path_never_the_value() {
        let ctx = AuthContext::new("dev".to_string(), false, "s".to_string());
        let manifest = serde_json::json!({
            "name": "core",
            "nodes": [{ "name": "r1", "model": "hunter2-s3cret" }],
        });
        let Err(error) = check_manifest(&ctx, &request(manifest, None)) else {
            panic!("parsed");
        };
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::RequestUnreadable {
                what: "manifest",
                refusal: InvalidRequest::Invalid {
                    path: Some("nodes[0].model".to_owned())
                },
            })
        );
    }
    fn request(
        manifest: serde_json::Value,
        file_contents: Option<serde_json::Value>,
    ) -> RedeployRequest {
        RedeployRequest {
            lab_id: "lab-0001".to_owned(),
            node_name: "r1".to_owned(),
            manifest,
            secrets: None,
            as_written: None,
            file_contents,
        }
    }

    /// The manifest check a redeploy of `r1` refuses `manifest` for, before anything is torn
    /// down.
    fn redeploy_refusal(manifest: &serde_json::Value) -> TopologyError {
        let ctx = AuthContext::new("dev".to_string(), false, "s".to_string());
        let Err(error) = check_manifest(&ctx, &request(manifest.clone(), None)) else {
            panic!("a redeploy of {manifest} was not refused");
        };
        check_refusal(&error)
    }

    /// The `TopologyError` of the manifest check that refused a redeploy with `error`.
    fn check_refusal(error: &anyhow::Error) -> TopologyError {
        let Some(refusal @ ServiceError::ManifestCheckFailed { refusal: check, .. }) =
            ServiceError::find(error)
        else {
            panic!("not a manifest check's refusal: {error:#}");
        };
        assert_eq!(refusal.category(), Some(ServiceCategory::Invalid));
        check.clone()
    }

    #[test]
    fn lab_settings_009_a_redeploy_is_refused_before_teardown_for_a_lab_name_the_rule_refuses() {
        let error = redeploy_refusal(&serde_json::json!({
            "name": "../x y",
            "nodes": [{ "name": "r1", "model": "ubuntu_linux" }],
        }));
        assert!(
            matches!(
                &error,
                TopologyError::NameNotPlain { what: "lab", name, .. } if name == "../x y"
            ),
            "{error}"
        );
    }

    #[test]
    fn lab_settings_010_a_redeploy_is_refused_before_teardown_for_a_ready_timeout_of_0() {
        let error = redeploy_refusal(&serde_json::json!({
            "name": "core",
            "ready_timeout": 0,
            "nodes": [{ "name": "r1", "model": "ubuntu_linux" }],
        }));
        assert_eq!(error, TopologyError::ReadyTimeoutZero);
    }

    #[test]
    fn lab_settings_012_a_redeploy_is_refused_before_teardown_for_a_ready_timeout_over_the_maximum()
    {
        let error = redeploy_refusal(&serde_json::json!({
            "name": "core",
            "ready_timeout": u64::MAX,
            "nodes": [{ "name": "r1", "model": "ubuntu_linux" }],
        }));
        assert_eq!(
            error,
            TopologyError::ReadyTimeoutTooLong { seconds: u64::MAX }
        );
    }

    #[test]
    fn node_settings_015_a_redeploy_is_refused_before_teardown_for_a_setting_of_0() {
        let error = redeploy_refusal(&serde_json::json!({
            "name": "core",
            "nodes": [{ "name": "r1", "model": "ubuntu_linux", "memory": 0 }],
        }));
        assert_eq!(
            error,
            TopologyError::ZeroSetting {
                node: "r1".to_owned(),
                field: "memory"
            }
        );
    }

    #[test]
    fn boot_settings_006_a_redeploy_is_refused_before_teardown_for_an_environment_variable_key_not_a_name()
     {
        let error = redeploy_refusal(&serde_json::json!({
            "name": "core",
            "nodes": [{
                "name": "r1",
                "model": "ubuntu_linux",
                "environment_variables": ["1BAD KEY=x"],
            }],
        }));
        assert!(
            matches!(
                &error,
                TopologyError::EnvironmentVariableKeyStartsWithDigit { key, node }
                    if key == "1BAD KEY" && node == "r1"
            ),
            "{error}"
        );
    }

    #[test]
    fn manifest_links_022_a_redeploy_is_refused_before_teardown_for_two_bridges_with_one_name() {
        let error = redeploy_refusal(&serde_json::json!({
            "name": "core",
            "nodes": [
                { "name": "r1", "model": "ubuntu_linux" },
                { "name": "r2", "model": "ubuntu_linux" },
            ],
            "bridges": [
                { "name": "br1", "links": ["r1::eth1", "r2::eth1"] },
                { "name": "br1", "links": ["r1::eth2", "r2::eth2"] },
            ],
        }));
        assert_eq!(
            error,
            TopologyError::DuplicateBridge {
                bridge: "br1".to_owned()
            }
        );
    }

    #[test]
    fn manifest_links_023_a_redeploy_is_refused_before_teardown_for_a_bridge_name_the_rule_refuses()
    {
        let error = redeploy_refusal(&serde_json::json!({
            "name": "core",
            "nodes": [
                { "name": "r1", "model": "ubuntu_linux" },
                { "name": "r2", "model": "ubuntu_linux" },
            ],
            "bridges": [
                { "name": "br/1", "links": ["r1::eth1", "r2::eth1"] },
            ],
        }));
        assert_eq!(
            error,
            TopologyError::BridgeNameNotAllowed {
                bridge: "br/1".to_owned(),
                reason: Box::new(TopologyError::NameNotPlain {
                    what: "node",
                    name: "br/1".to_owned(),
                    max_len: 63,
                }),
            }
        );
    }

    #[test]
    fn node_naming_008_a_redeploy_is_refused_before_teardown_for_node_names_that_differ_only_in_case()
     {
        let error = redeploy_refusal(&serde_json::json!({
            "name": "core",
            "nodes": [
                { "name": "r1", "model": "ubuntu_linux" },
                { "name": "R1", "model": "ubuntu_linux" },
            ],
        }));
        assert_eq!(
            error,
            TopologyError::NodeNamesDifferInCase {
                node: "R1".to_owned(),
                other: "r1".to_owned(),
            }
        );
    }

    #[test]
    fn boot_settings_007_a_redeploy_is_refused_before_teardown_for_a_flatcar_text_file_mode_not_octal()
     {
        let ctx = AuthContext::new("dev".to_string(), false, "s".to_string());
        let manifest = serde_json::json!({
            "name": "core",
            "nodes": [{ "name": "r1", "model": "flatcar_linux" }],
        });
        let file_contents = serde_json::json!({
            "r1": {
                "text_files": [{
                    "content": "aGk=",
                    "dst": "/etc/motd",
                    "user": "core",
                    "group": "core",
                    "permissions": 999,
                }],
            },
        });
        let Err(error) = check_manifest(&ctx, &request(manifest, Some(file_contents))) else {
            panic!("a redeploy of a text file mode 999 was not refused");
        };
        assert_eq!(
            check_refusal(&error),
            TopologyError::TextFilePermissions {
                node: "r1".to_owned(),
                dst: "/etc/motd".to_owned(),
                mode: SharedError::ModeNotOctal { digits: 999 },
            }
        );
    }

    #[tokio::test]
    async fn cluster_redeploy_010_a_node_is_rebuilt_with_the_stored_cluster_membership() {
        let (db, lab) = lab_with(&[
            ("cp1", NodeModel::TalosLinux),
            ("w1", NodeModel::TalosLinux),
            ("w2", NodeModel::TalosLinux),
        ])
        .await;
        let nodes = r#"
[[nodes]]
name = "cp1"
model = "talos_linux"

[[nodes]]
name = "w1"
model = "talos_linux"

[[nodes]]
name = "w2"
model = "talos_linux"
"#;
        store(
            &db,
            &lab,
            &format!(
                r#"name = "core"

[[talos_clusters]]
name = "k1"
controllers = ["cp1"]
workers = ["w1", "w2"]
{nodes}"#
            ),
        )
        .await;
        // The manifest sent gives w1 the same cluster and role, but leaves w2 out of it.
        let manifest: topology::Manifest = toml::from_str(&format!(
            r#"name = "core"

[[talos_clusters]]
name = "k1"
controllers = ["cp1"]
workers = ["w1"]
{nodes}"#
        ))
        .unwrap();

        let clusters = stored_talos_clusters(&db, lab.id.as_ref().unwrap(), &manifest, "w1")
            .await
            .unwrap();

        let (cluster, role) = find_talos_cluster(&clusters, "w1").unwrap();
        assert_eq!(role, data::ClusterRole::Worker);
        assert_eq!(cluster.workers, ["w1", "w2"]);
    }

    #[test]
    fn owner_login_020_a_redeployed_node_records_the_owner_or_ftdvs_admin() {
        assert_eq!(
            login_after_redeploy("alice", NodeModel::UbuntuLinux, false).unwrap(),
            Some("alice".to_owned())
        );
        assert_eq!(
            login_after_redeploy("alice", NodeModel::CiscoFtd, false).unwrap(),
            Some("admin".to_owned())
        );
        // A custom config writes no account; the SSH config names the owner.
        assert_eq!(
            login_after_redeploy("alice", NodeModel::AristaEos, true).unwrap(),
            None
        );
    }

    #[test]
    fn a_redeploy_is_refused_before_teardown_for_a_name_the_model_cannot_take() {
        let error =
            login_after_redeploy(&"w".repeat(21), NodeModel::WindowsServer, false).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ServiceError>(),
                Some(ServiceError::LoginNameTooLong { node, max_length: 20, .. }) if node == "n1"
            ),
            "{error}"
        );
    }

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
            ssh_host_key: None,
            ready_check: ReadyCheck::default(),
            ssh_username: None,
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

    #[tokio::test]
    async fn redeploy_takes_the_lab_and_its_network_from_the_database() {
        let (db, lab) = lab_with(&[("vm1", NodeModel::AlpineLinux)]).await;
        let node = &db::list_nodes_by_lab(&db, lab.id.clone().unwrap())
            .await
            .unwrap()[0];
        db::update_node_ssh_username(&db, node.id.clone().unwrap(), Some("alice"))
            .await
            .unwrap();

        let lab_info = lab_info::load_lab_info(&db, &lab).await.unwrap();

        // The other nodes' accounts survive a redeploy of one node.
        assert_eq!(lab_info.ssh_username("vm1"), Some("alice"));
        let network = lab_management_network(&lab_info);
        let expected = mgmt_net(false);
        assert_eq!(network.v4.prefix, expected.v4.prefix);
        assert_eq!(network.v4.first, expected.v4.first);
        assert_eq!(network.v4.boot_server, expected.v4.boot_server);
        assert!(network.v6.is_none());
    }

    /// A database with two `arista_eos` container images, 4.33 the default, and a VM image
    /// 4.35 of the same model.
    async fn database_with_ceos_versions() -> db::Database {
        let database = db::open_in_memory().await.unwrap();
        db::apply_schema(&database).await.unwrap();
        let ceos = ModelKind::new(NodeModel::AristaEos, Some(data::NodeKind::Container)).unwrap();
        for (model_kind, version, default) in [
            (ceos, "4.33", true),
            (ceos, "4.34", false),
            (ModelKind::default_for(NodeModel::AristaEos), "4.35", true),
        ] {
            let image = data::NodeConfig {
                version: version.to_owned(),
                default,
                ..model_kind.node_config()
            };
            let request = if default {
                DefaultRequest::Make
            } else {
                DefaultRequest::Keep
            };
            db::upsert_node_image(&database, image, request)
                .await
                .unwrap();
        }
        database
    }

    #[tokio::test]
    async fn image_selection_014_a_redeploy_rebuilds_a_node_from_the_manifests_version_of_its_model_and_kind()
     {
        let database = database_with_ceos_versions().await;
        let ceos = ModelKind::new(NodeModel::AristaEos, Some(data::NodeKind::Container)).unwrap();

        let image = redeploy_image(&database, ceos, Some("4.34")).await.unwrap();
        assert_eq!(image.version, "4.34");
        assert_eq!(image.kind(), data::NodeKind::Container);
        // Without a version, the default of the model and kind, as `up` takes.
        let image = redeploy_image(&database, ceos, None).await.unwrap();
        assert_eq!(image.version, "4.33");
        // Another kind's version is never the node's.
        let error = redeploy_image(&database, ceos, Some("4.35"))
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::ImageVersionNotFound {
                model_kind: ceos,
                version: "4.35".to_owned(),
            })
        );
    }
}
