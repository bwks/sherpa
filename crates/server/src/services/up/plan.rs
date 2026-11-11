//! Phase 2: validating the manifest against the server, and planning the lab.

use anyhow::{Context, Result, anyhow};
use tracing::instrument;

use super::context::LabBuild;
use super::nodes::{get_node_image, process_manifest_nodes};
use crate::services::admission::{self, Admission};
use crate::services::guest_account;
use crate::services::guest_account::GuestAccount;
use crate::services::image_lifecycle::{
    ImageLease, check_container_image, lease_image, validate_disk_method,
};
use crate::services::progress::ProgressSender;
use crate::validate::{
    validate_and_resolve_node_versions, validate_environment_variables, validate_node_ztp,
};

use shared::data;
use shared::data::{NodeCpuPins, NodeKind, NodeSshUsername, StatusKind};
use shared::validators::{effective_data_interface_count, effective_ztp};
use topology::validate::{
    check_cpu_pin_overlap, check_device_count, check_duplicate_device,
    check_duplicate_interface_link, check_interface_bounds, check_link_impairments,
    check_no_p2p_field, check_node_names, check_performance_tuning, check_ptx_platform,
    check_text_file_modes, check_watchdog, validate_talos_clusters,
};
use topology::{process_manifest_bridges, process_manifest_links};

/// What validation decided before anything is created: the nodes and their images, the image
/// leases that keep those images until the nodes use them, the owner's account on the nodes, the
/// links and bridges, and the nodes' admission to the server's memory and CPUs.
pub(super) struct LabPlan {
    pub(super) node_images: Vec<data::NodeConfig>,
    pub(super) nodes_expanded: Vec<topology::NodeExpanded>,
    pub(super) selected_images: Vec<data::NodeConfig>,
    pub(super) image_guards: Vec<ImageLease>,
    pub(super) container_image_guards: Vec<ImageLease>,
    pub(super) personal_account: Option<GuestAccount>,
    pub(super) ssh_usernames: Vec<NodeSshUsername>,
    pub(super) links_detailed: Vec<topology::LinkDetailed>,
    pub(super) bridges_detailed: Vec<topology::BridgeDetailed>,
    /// Holds the nodes' memory and CPU pins against other operations until `up` returns.
    pub(super) _admission: Admission,
}

/// The images the lab's nodes use, sorted and deduplicated, with their leases: disk images'
/// apart from container images', since the two are released at different times.
struct ImageLeases {
    selected_images: Vec<data::NodeConfig>,
    image_guards: Vec<ImageLease>,
    container_image_guards: Vec<ImageLease>,
}

/// Validate the manifest against the server and plan the lab (phase 2). Creates nothing.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn plan_lab(
    build: &LabBuild<'_>,
    manifest: &topology::Manifest,
    db_user: &data::DbUser,
    node_images: Vec<data::NodeConfig>,
) -> Result<LabPlan> {
    let LabBuild {
        lab_id,
        db,
        docker_conn,
        qemu_conn,
        config,
        progress,
        ..
    } = *build;

    tracing::info!(
        lab_id = %lab_id,
        lab_name = %manifest.name,
        "Validating lab manifest"
    );

    tracing::debug!(lab_id = %lab_id, node_images = node_images.len(), "Fetched node configs from database");

    check_manifest_nodes(manifest)?;

    // Version & Image Validators (CRITICAL ERROR - fail fast on validation failure)
    let validated_nodes =
        validate_and_resolve_node_versions(&manifest.nodes, &node_images, &config.images_dir)
            .context("Manifest validation failed: version/image validation")?;

    let nodes_expanded = process_manifest_nodes(&validated_nodes);
    let ImageLeases {
        selected_images,
        image_guards,
        container_image_guards,
    } = lease_images(db, &nodes_expanded, &node_images).await?;
    for node in &nodes_expanded {
        let image = get_node_image(node.model, node.version.as_deref(), &node_images)?;
        validate_disk_method(
            node.boot_disk_clone_method,
            &image,
            &config.images_dir,
            node.boot_disk_size,
        )
        .with_context(|| format!("Disk provisioning validation failed for node {}", node.name))?;
        validate_node_ztp(
            node,
            &image,
            manifest.talos_clusters.as_deref().unwrap_or_default(),
        )?;
        if image.kind == NodeKind::Container {
            let repo = node.image.as_deref().with_context(|| {
                format!(
                    "Container node '{}' has no image after validation",
                    node.name
                )
            })?;
            check_container_image(docker_conn, &node.name, node.model, repo, &image.version)
                .await?;
        }
    }

    let (personal_account, ssh_usernames) =
        owner_accounts(manifest, &nodes_expanded, &node_images, db_user, progress)?;

    let (links_detailed, bridges_detailed) = process_links(manifest, &nodes_expanded, lab_id)?;

    check_node_limits(
        &nodes_expanded,
        &node_images,
        &links_detailed,
        &bridges_detailed,
    )?;
    let manifest_pins: Vec<NodeCpuPins> = nodes_expanded
        .iter()
        .filter_map(|node| {
            node.performance_tuning
                .as_ref()
                .map(|tuning| NodeCpuPins::of(&node.name, tuning))
        })
        .collect();
    check_cpu_pin_overlap(&manifest_pins).context("CPU pinning validation failed")?;
    let images = nodes_expanded
        .iter()
        .map(|node| get_node_image(node.model, node.version.as_deref(), &node_images))
        .collect::<Result<Vec<_>>>()?;
    let admitted: Vec<(&topology::NodeExpanded, &data::NodeConfig)> =
        nodes_expanded.iter().zip(&images).collect();
    let admission =
        admission::admit_lab_nodes(db, qemu_conn, config.host_memory, lab_id, &admitted).await?;

    // Connection Validators
    if !links_detailed.is_empty() {
        check_duplicate_interface_link(&links_detailed, &bridges_detailed)
            .context("Duplicate interface link validation failed")?;
    }

    let _ = progress.send_status("Manifest validation complete".to_string(), StatusKind::Done);
    tracing::info!(lab_id = %lab_id, "Manifest validation completed successfully");
    Ok(LabPlan {
        node_images,
        nodes_expanded,
        selected_images,
        image_guards,
        container_image_guards,
        personal_account,
        ssh_usernames,
        links_detailed,
        bridges_detailed,
        _admission: admission,
    })
}

/// The manifest's node checks that need nothing from the server: names, duplicates, count,
/// Talos clusters and environment variables (CRITICAL ERROR - fail fast on validation failure).
fn check_manifest_nodes(manifest: &topology::Manifest) -> Result<()> {
    check_node_names(&manifest.nodes).context("Manifest validation failed: node names")?;
    check_duplicate_device(&manifest.nodes)
        .context("Manifest validation failed: duplicate devices")?;
    check_device_count(&manifest.nodes).context("Manifest validation failed: node count")?;
    validate_talos_clusters(manifest).context("Manifest validation failed: talos clusters")?;

    for node in &manifest.nodes {
        if let Some(ref env_vars) = node.environment_variables {
            validate_environment_variables(env_vars, &node.name)
                .context("Manifest validation failed: environment variables")?;
        }
    }
    Ok(())
}

/// Lease every image the nodes use, and check that each is still the image validation chose.
///
/// Acquire in a stable order to avoid deadlocks between overlapping labs.
/// Leases stop deletion racing validation, database references and disk creation.
/// Disk image (VM and unikernel) leases end once every disk and domain exists: node
/// records and overlay backing files protect the images from then on, and a long
/// readiness wait must not block image pull, import or delete. Container image leases
/// last until the containers start, during readiness polling.
#[instrument(skip_all, level = "debug")]
async fn lease_images(
    db: &db::Database,
    nodes_expanded: &[topology::NodeExpanded],
    node_images: &[data::NodeConfig],
) -> Result<ImageLeases> {
    let mut selected_images = nodes_expanded
        .iter()
        .map(|node| get_node_image(node.model, node.version.as_deref(), node_images))
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
        let current = db::find_node_image_by_model_kind_version(
            db,
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
    Ok(ImageLeases {
        selected_images,
        image_guards,
        container_image_guards,
    })
}

/// The owner's account for the nodes that take it, and each such node's login. Every node must
/// take one of the owner's keys before anything is created.
fn owner_accounts(
    manifest: &topology::Manifest,
    nodes_expanded: &[topology::NodeExpanded],
    node_images: &[data::NodeConfig],
    db_user: &data::DbUser,
    progress: &ProgressSender,
) -> Result<(Option<GuestAccount>, Vec<NodeSshUsername>)> {
    let mut personal_nodes = Vec::new();
    if manifest.provision_current_user {
        for node in nodes_expanded {
            let image = get_node_image(node.model, node.version.as_deref(), node_images)?;
            if guest_account::provisions_owner(node, &image) {
                personal_nodes.push((node.name.clone(), image));
            } else if guest_account::supports_guest_account(
                image.model,
                effective_ztp(node.ztp, &image),
            ) {
                let _ = progress.send_status(
                    format!(
                        "Node '{}' has a custom ZTP config, so it gets no account for '{}'",
                        node.name, db_user.username
                    ),
                    StatusKind::Info,
                );
            }
        }
    }
    let personal_account = if personal_nodes.is_empty() {
        None
    } else {
        let account = guest_account::from_user(db_user)?;
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
        for (name, image) in &personal_nodes {
            let node_account = guest_account::node_account(account, name, image)?;
            if !node_account.skipped_keys.is_empty() {
                let _ = progress.send_status(
                    format!(
                        "Node '{name}' does not take these SSH keys of '{}': {}",
                        account.username,
                        node_account.skipped_keys.join(", ")
                    ),
                    StatusKind::Info,
                );
            }
            ssh_usernames.push(NodeSshUsername {
                node_name: name.clone(),
                username: node_account.account.username,
            });
        }
    }
    Ok((personal_account, ssh_usernames))
}

/// Each node's interface bounds, performance tuning, watchdog and platform, against its image.
fn check_node_limits(
    nodes_expanded: &[topology::NodeExpanded],
    node_images: &[data::NodeConfig],
    links_detailed: &[topology::LinkDetailed],
    bridges_detailed: &[topology::BridgeDetailed],
) -> Result<()> {
    for node in nodes_expanded {
        let node_image = get_node_image(node.model, node.version.as_deref(), node_images)
            .context(format!("Node config not found for model: {}", node.model))?;

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
            links_detailed,
            bridges_detailed,
        )
        .context(format!(
            "Interface bounds validation failed for node: {}",
            node.name
        ))?;

        check_performance_tuning(node, &node_image).context(format!(
            "Performance tuning validation failed for node: {}",
            node.name
        ))?;
        check_watchdog(node, &node_image).context(format!(
            "Watchdog validation failed for node: {}",
            node.name
        ))?;
        check_ptx_platform(node, &node_image).context(format!(
            "Platform validation failed for node: {}",
            node.name
        ))?;
    }
    Ok(())
}

/// The manifest's links and bridges, checked and expanded against the nodes.
fn process_links(
    manifest: &topology::Manifest,
    nodes_expanded: &[topology::NodeExpanded],
    lab_id: &str,
) -> Result<(Vec<topology::LinkDetailed>, Vec<topology::BridgeDetailed>)> {
    check_no_p2p_field(manifest.links.as_deref().unwrap_or_default())
        .context("Link validation failed")?;
    check_link_impairments(manifest.links.as_deref().unwrap_or_default())
        .context("Link validation failed")?;
    check_text_file_modes(&manifest.nodes).context("Text file validation failed")?;
    let links_detailed = process_manifest_links(
        manifest.links.as_deref().unwrap_or_default(),
        nodes_expanded,
    )
    .context("Failed to process manifest links")?;
    let bridges_detailed = process_manifest_bridges(
        manifest.bridges.as_deref().unwrap_or_default(),
        nodes_expanded,
        lab_id,
    )
    .context("Failed to process manifest bridges")?;

    tracing::info!(
        lab_id = %lab_id,
        nodes = nodes_expanded.len(),
        links = links_detailed.len(),
        bridges = bridges_detailed.len(),
        "Processed manifest structures"
    );
    Ok((links_detailed, bridges_detailed))
}
