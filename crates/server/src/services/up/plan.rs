//! Phase 2: validating the manifest against the server, and planning the lab.

use anyhow::{Context, Result};
use tracing::instrument;

use super::context::LabBuild;
use super::nodes::{get_node_image, process_manifest_nodes};
use crate::auth::context::AuthContext;
use crate::services::admission::{self, Admission};
use crate::services::error::ServiceError;
use crate::services::guest_account;
use crate::services::guest_account::GuestAccount;
use crate::services::image_lifecycle::{
    ImageLease, check_container_image, lease_image, validate_disk_method,
};
use crate::services::progress::ProgressSender;
use crate::services::sandbox::{self, SandboxBase};
use crate::validate::{
    sandbox_volume_sources, validate_and_resolve_node_versions, validate_node_ztp,
    validate_sandbox_volumes,
};

use shared::data;
use shared::data::{NodeCpuPins, NodeKind, NodeSshUsername, StatusKind};
use shared::validators::{effective_data_interface_count, effective_ztp};
use topology::validate::{
    check_bridges, check_cpu_pin_overlap, check_device_count, check_duplicate_device,
    check_duplicate_interface_link, check_interface_bounds, check_link_impairments,
    check_links_to_itself, check_node_file_settings, check_node_kind_settings,
    check_node_model_kind, check_node_names, check_nonzero_node_settings, check_ptx_platform,
    check_text_file_modes, check_vm_tuning, check_windows_startup_script_names,
    validate_environment_variables, validate_talos_clusters,
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
    pub(super) sandbox_base: Option<SandboxBase>,
    pub(super) container_image_guards: Vec<ImageLease>,
    pub(super) owner_account: GuestAccount,
    pub(super) ssh_usernames: Vec<NodeSshUsername>,
    pub(super) links_detailed: Vec<topology::LinkDetailed>,
    pub(super) bridges_detailed: Vec<topology::BridgeDetailed>,
    pub(super) _admission: Admission,
}

/// Every node image the server has, and the manifest's nodes checked against them, with their
/// versions and container images resolved.
pub(super) struct CheckedNodes {
    pub(super) node_images: Vec<data::NodeConfig>,
    pub(super) nodes: Vec<topology::Node>,
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
    requester: &AuthContext,
    manifest: &topology::Manifest,
    owner_account: GuestAccount,
    checked: CheckedNodes,
) -> Result<LabPlan> {
    let CheckedNodes {
        node_images,
        nodes: validated_nodes,
    } = checked;
    let LabBuild {
        lab_id,
        db,
        docker_conn,
        config,
        progress,
        ..
    } = *build;

    tracing::info!(lab_id = %lab_id, lab_name = %manifest.name, "Validating lab manifest");

    tracing::debug!(lab_id = %lab_id, node_images = node_images.len(), "Fetched node configs from database");

    let nodes_expanded = process_manifest_nodes(&validated_nodes)?;
    for (node, expanded) in validated_nodes.iter().zip(&nodes_expanded) {
        check_node_kind_settings(expanded)
            .context("Manifest validation failed: node settings")
            .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
        let image = get_node_image(
            expanded.model_kind,
            expanded.version.as_deref(),
            &node_images,
        )?;
        check_node_file_settings(node, &image)
            .context("Manifest validation failed: node settings")
            .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    }
    let ImageLeases {
        selected_images,
        image_guards,
        container_image_guards,
    } = lease_images(db, &nodes_expanded, &node_images).await?;
    let model_kinds = selected_images
        .iter()
        .map(data::NodeConfig::model_kind)
        .collect::<Result<Vec<_>>>()?;
    let sandbox_base = sandbox::sandbox_base(db, &config.images_dir, &model_kinds).await?;
    let mut volume_sources = Vec::new();
    for node in &nodes_expanded {
        let image = get_node_image(node.model_kind, node.version.as_deref(), &node_images)?;
        volume_sources.extend(sandbox_volume_sources(node, &image));
        validate_disk_method(
            &node.name,
            node.boot_disk.clone_method,
            &image,
            &config.images_dir,
            node.boot_disk.size,
        )?;
        validate_node_ztp(
            node,
            &image,
            manifest.talos_clusters.as_deref().unwrap_or_default(),
        )?;
        if image.kind() == NodeKind::Container {
            let repo = node.image.as_deref().with_context(|| {
                format!(
                    "Container node '{}' has no image after validation",
                    node.name
                )
            })?;
            check_container_image(docker_conn, &node.name, image.model, repo, &image.version)
                .await?;
        }
    }

    validate_sandbox_volumes(volume_sources).await?;

    let ssh_usernames = node_logins(&owner_account, &nodes_expanded, &node_images, progress)?;

    let (links_detailed, bridges_detailed) = process_links(manifest, &nodes_expanded, lab_id)
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;

    check_node_limits(
        &nodes_expanded,
        &node_images,
        &links_detailed,
        &bridges_detailed,
    )?;
    let admission = admit_nodes(build, requester, &nodes_expanded, &node_images).await?;

    let _ = progress.send_status("Manifest validation complete".to_string(), StatusKind::Done);
    tracing::info!(lab_id = %lab_id, "Manifest validation completed successfully");
    Ok(LabPlan {
        node_images,
        nodes_expanded,
        selected_images,
        image_guards,
        sandbox_base,
        container_image_guards,
        owner_account,
        ssh_usernames,
        links_detailed,
        bridges_detailed,
        _admission: admission,
    })
}

/// Admit the lab's nodes to the server's CPUs and memory, once their pins do not overlap each
/// other's. A warning for each overlap their stored pins already had goes to the user.
async fn admit_nodes(
    build: &LabBuild<'_>,
    requester: &AuthContext,
    nodes_expanded: &[topology::NodeExpanded],
    node_images: &[data::NodeConfig],
) -> Result<Admission> {
    let manifest_pins: Vec<NodeCpuPins> = nodes_expanded
        .iter()
        .map(|node| NodeCpuPins::of(&node.name, &node.cpu, &node.boot_disk))
        .filter(|pins| !pins.is_empty())
        .collect();
    check_cpu_pin_overlap(&manifest_pins)
        .context("CPU pinning validation failed")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    let images = nodes_expanded
        .iter()
        .map(|node| get_node_image(node.model_kind, node.version.as_deref(), node_images))
        .collect::<Result<Vec<_>>>()?;
    let admitted: Vec<(&topology::NodeExpanded, &data::NodeConfig)> =
        nodes_expanded.iter().zip(&images).collect();
    let admission = admission::admit_lab_nodes(
        build.db,
        build.qemu_conn,
        build.docker_conn,
        requester,
        build.config.host_memory,
        build.lab_id,
        &admitted,
    )
    .await?;
    for warning in admission.shared_cpus() {
        let _ = build
            .progress
            .send_status(format!("Warning: {warning}"), StatusKind::Info);
    }
    Ok(admission)
}

/// Check the manifest's nodes, its links for one to itself and its bridges' names and links,
/// and that the server has every node's image: a model and kind with an image, the version, and
/// a VM's or unikernel's disk or a container's repo. Returns the images with the nodes, their
/// versions and container images resolved.
///
/// Needs only the database's node images and the images directory, so `up` runs it before it
/// connects to libvirt or Docker: a missing image gets the same answer whether or not the
/// hypervisor is reachable.
pub(super) fn check_node_images(
    manifest: &topology::Manifest,
    node_images: Vec<data::NodeConfig>,
    images_dir: &str,
) -> Result<CheckedNodes> {
    check_manifest_nodes(manifest).map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    let nodes = validate_and_resolve_node_versions(&manifest.nodes, &node_images, images_dir)?;
    Ok(CheckedNodes { node_images, nodes })
}

/// Run the manifest checks that need nothing from the server: node names, duplicates, count,
/// kinds, zero settings, Talos clusters, environment variables, startup scripts, a link to
/// itself and the bridges' names and links.
fn check_manifest_nodes(manifest: &topology::Manifest) -> Result<()> {
    check_node_names(&manifest.nodes).context("Manifest validation failed: node names")?;
    for node in &manifest.nodes {
        check_node_model_kind(node).context("Manifest validation failed: node kind")?;
    }
    check_duplicate_device(&manifest.nodes)
        .context("Manifest validation failed: duplicate nodes")?;
    check_device_count(&manifest.nodes).context("Manifest validation failed: node count")?;
    check_nonzero_node_settings(&manifest.nodes)
        .context("Manifest validation failed: node settings")?;
    validate_talos_clusters(manifest).context("Manifest validation failed: talos clusters")?;
    check_links_to_itself(manifest.links.as_deref().unwrap_or_default())
        .context("Manifest validation failed: links")?;
    check_bridges(manifest.bridges.as_deref().unwrap_or_default())
        .context("Manifest validation failed: bridges")?;

    validate_environment_variables(&manifest.nodes)
        .context("Manifest validation failed: environment variables")?;
    check_windows_startup_script_names(&manifest.nodes)
        .context("Manifest validation failed: startup scripts")?;
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
        .map(|node| get_node_image(node.model_kind, node.version.as_deref(), node_images))
        .collect::<Result<Vec<_>>>()?;
    selected_images.sort_by_key(|image| {
        (
            image.model.to_string(),
            image.kind().to_string(),
            image.version.clone(),
        )
    });
    selected_images
        .dedup_by(|a, b| a.model == b.model && a.kind() == b.kind() && a.version == b.version);
    let mut image_guards = Vec::new();
    let mut container_image_guards = Vec::new();
    for image in &selected_images {
        let lease = lease_image(image.model_kind()?, &image.version).await;
        if image.kind() == data::NodeKind::Container {
            container_image_guards.push(lease);
        } else {
            image_guards.push(lease);
        }
        let current = db::find_node_image_by_model_kind_version(
            db,
            &image.model,
            &image.kind(),
            &image.version,
        )
        .await?
        .ok_or_else(|| {
            ServiceError::Conflict("Selected image was deleted before provisioning".to_owned())
        })?;
        if current.id != image.id {
            return Err(ServiceError::Conflict(
                "Selected image changed before provisioning; retry startup".to_owned(),
            )
            .into());
        }
    }
    Ok(ImageLeases {
        selected_images,
        image_guards,
        container_image_guards,
    })
}

/// The login of each node that records one: the owner's name on every node with a generated
/// login, and `admin` on Cisco `FTDv`. Every node must take the owner's name and one of their
/// keys before anything is created.
fn node_logins(
    account: &GuestAccount,
    nodes_expanded: &[topology::NodeExpanded],
    node_images: &[data::NodeConfig],
    progress: &ProgressSender,
) -> Result<Vec<NodeSshUsername>> {
    let mut ssh_usernames = Vec::new();
    for node in nodes_expanded {
        let image = get_node_image(node.model_kind, node.version.as_deref(), node_images)?;
        let username = if guest_account::provisions_owner(node, &image) {
            let node_account = guest_account::node_account(account, &node.name, &image)?;
            if !node_account.skipped_keys.is_empty() {
                let _ = progress.send_status(
                    format!(
                        "Node '{}' does not take these SSH keys of '{}': {}",
                        node.name,
                        account.username,
                        node_account.skipped_keys.join(", ")
                    ),
                    StatusKind::Info,
                );
            }
            node_account.account.username
        } else if let Some(login) = guest_account::fixed_login(image.model) {
            login.to_owned()
        } else {
            if guest_account::supports_guest_account(
                image.model,
                effective_ztp(node.ztp.method, &image),
            ) {
                let _ = progress.send_status(
                    format!(
                        "Node '{}' has a custom ZTP config, so it gets no account for '{}'",
                        node.name, account.username
                    ),
                    StatusKind::Info,
                );
            }
            continue;
        };
        ssh_usernames.push(NodeSshUsername {
            node_name: node.name.clone(),
            username,
        });
    }
    Ok(ssh_usernames)
}

/// Each node's interface bounds, VM tuning and platform, against its image.
fn check_node_limits(
    nodes_expanded: &[topology::NodeExpanded],
    node_images: &[data::NodeConfig],
    links_detailed: &[topology::LinkDetailed],
    bridges_detailed: &[topology::BridgeDetailed],
) -> Result<()> {
    for node in nodes_expanded {
        let node_image = get_node_image(node.model_kind, node.version.as_deref(), node_images)?;

        let data_interface_count =
            effective_data_interface_count(&node.name, node.interfaces.data_count, &node_image)
                .context(format!(
                    "Data interface count validation failed for node: {}",
                    node.name
                ))
                .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;

        check_interface_bounds(
            &node.name,
            &node_image.model,
            data_interface_count,
            node_image.interfaces.reserved_count,
            links_detailed,
            bridges_detailed,
        )
        .context(format!(
            "Interface bounds validation failed for node: {}",
            node.name
        ))
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;

        check_vm_tuning(node, &node_image)
            .context(format!(
                "VM tuning validation failed for node: {}",
                node.name
            ))
            .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
        check_ptx_platform(node, &node_image)
            .context(format!(
                "Platform validation failed for node: {}",
                node.name
            ))
            .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    }
    Ok(())
}

/// The manifest's links and bridges, checked and expanded against the nodes, with no interface
/// used by two of them.
fn process_links(
    manifest: &topology::Manifest,
    nodes_expanded: &[topology::NodeExpanded],
    lab_id: &str,
) -> Result<(Vec<topology::LinkDetailed>, Vec<topology::BridgeDetailed>)> {
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
    check_duplicate_interface_link(&links_detailed, &bridges_detailed)
        .context("Duplicate interface link validation failed")?;

    tracing::info!(
        lab_id = %lab_id,
        nodes = nodes_expanded.len(),
        links = links_detailed.len(),
        bridges = bridges_detailed.len(),
        "Processed manifest structures"
    );
    Ok((links_detailed, bridges_detailed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::guest_account::tests::test_account;
    use shared::data::{ModelKind, NodeModel};
    use tokio::sync::mpsc;
    use topology::{Connection, TopologyError};

    const OWNER_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh";

    /// `up`'s node logins for `nodes` owned by `username`, each of its model's default image.
    fn logins(username: &str, nodes: &[topology::NodeExpanded]) -> Result<Vec<NodeSshUsername>> {
        let images: Vec<data::NodeConfig> = nodes
            .iter()
            .map(|node| data::NodeConfig {
                default: true,
                ..node.model_kind.node_config()
            })
            .collect();
        let (tx, _rx) = mpsc::unbounded_channel();
        node_logins(
            &test_account(username, &[OWNER_KEY]),
            nodes,
            &images,
            &ProgressSender::new(tx),
        )
    }

    fn node(name: &str, model: NodeModel) -> topology::NodeExpanded {
        topology::NodeExpanded {
            name: name.to_owned(),
            model_kind: ModelKind::default_for(model),
            ..Default::default()
        }
    }

    #[test]
    fn owner_login_019_every_node_with_a_generated_login_records_the_owner_and_ftdv_admin() {
        let mut custom = node("eos2", NodeModel::AristaEos);
        custom.ztp.config = Some("hostname eos2\n".to_owned());
        let nodes = [
            node("vm1", NodeModel::UbuntuLinux),
            node("eos1", NodeModel::AristaEos),
            node("fw1", NodeModel::CiscoFtd),
            node("web", NodeModel::PostgresqlDb),
            custom,
        ];

        let logins = logins("alice", &nodes).unwrap();

        let logins: Vec<(&str, &str)> = logins
            .iter()
            .map(|entry| (entry.node_name.as_str(), entry.username.as_str()))
            .collect();
        assert_eq!(
            logins,
            [("vm1", "alice"), ("eos1", "alice"), ("fw1", "admin")]
        );
    }

    #[test]
    fn manifest_links_014_up_refuses_a_manifest_of_bridges_alone_with_one_interface_on_two_bridges()
    {
        let nodes = [
            node("r1", NodeModel::UbuntuLinux),
            node("r2", NodeModel::UbuntuLinux),
        ];
        let manifest = topology::Manifest {
            name: "lab1".to_owned(),
            bridges: Some(vec![
                topology::Bridge {
                    name: "b1".to_owned(),
                    links: vec!["r1::eth1".to_owned(), "r2::eth1".to_owned()],
                },
                topology::Bridge {
                    name: "b2".to_owned(),
                    links: vec!["r1::eth1".to_owned(), "r2::eth2".to_owned()],
                },
            ]),
            ..Default::default()
        };

        let error = process_links(&manifest, &nodes, "abcd1234").unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<TopologyError>(),
                Some(TopologyError::InterfaceInUse {
                    connection: Connection::Bridge { name },
                    node,
                    ..
                }) if name == "b2" && node == "r1"
            ),
            "{error:#}"
        );
    }

    #[test]
    fn manifest_links_021_up_refuses_a_link_from_a_node_to_itself_before_libvirt_or_docker() {
        let manifest = topology::Manifest {
            name: "lab1".to_owned(),
            nodes: vec![topology::Node {
                name: "r1".to_owned(),
                ..Default::default()
            }],
            links: Some(vec![topology::Link2 {
                src: "r1::eth1".to_owned(),
                dst: "r1::eth2".to_owned(),
                impairment: None,
            }]),
            ..Default::default()
        };

        let error = check_manifest_nodes(&manifest).unwrap_err();

        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::LinkToItself {
                src: "r1::eth1".to_owned(),
                dst: "r1::eth2".to_owned(),
                node: "r1".to_owned(),
            }),
            "{error:#}"
        );
    }

    #[test]
    fn up_is_refused_for_a_name_a_node_model_cannot_take() {
        let error = logins(
            "a-very-long-owner-name",
            &[
                node("vm1", NodeModel::UbuntuLinux),
                node("win1", NodeModel::WindowsServer),
            ],
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::LoginNameTooLong {
                node: "win1".to_owned(),
                model_kind: ModelKind::default_for(NodeModel::WindowsServer),
                username: "a-very-long-owner-name".to_owned(),
                max_length: 20,
                length: 22,
            })
        );
    }
}
