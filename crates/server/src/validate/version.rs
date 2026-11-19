use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashMap;
use std::path::Path;

use shared::data::{KindSettings, ModelKind, NodeConfig, NodeKind, NodeModel};
use shared::util::image_filename;
use topology::Node;

/// Validate and resolve node versions and images
///
/// This function performs three key validations:
/// 1. Resolves version from `node_image` if not set in manifest
/// 2. Validates version exists in database for the model and kind
/// 3. Resolves a container node's image repo, or verifies a VM's or unikernel's disk exists
///
/// Whether Docker has a container node's image is asked of Docker by the caller, with the
/// image and version resolved here.
///
/// # Arguments
/// * `nodes` - List of nodes from manifest
/// * `node_images` - Pre-fetched list of all node configs from DB
/// * `images_dir` - Base directory for VM images
///
/// # Returns
/// Updated nodes with version populated, and for containers the image repo they run
///
/// # Errors
/// Returns error if:
/// - The node sets a kind its model does not take
/// - Version not found in `node_image` for model and kind
/// - Container node has no image repo
/// - VM disk file not found on filesystem
pub fn validate_and_resolve_node_versions(
    nodes: &[Node],
    node_images: &[NodeConfig],
    images_dir: &str,
) -> Result<Vec<Node>> {
    let mut validated_nodes = Vec::new();

    // Build lookup map for faster access, preferring default images per model and kind
    let mut config_map: HashMap<(NodeModel, NodeKind), &NodeConfig> = HashMap::new();
    for config in node_images {
        config_map
            .entry((config.model, config.kind()))
            .and_modify(|existing| {
                if config.default && !existing.default {
                    *existing = config;
                }
            })
            .or_insert(config);
    }

    for node in nodes {
        let mut updated_node = node.clone();

        // Step 1: Get node_image for this model and kind
        let model_kind = ModelKind::new(node.model, node.kind)
            .with_context(|| format!("Manifest - node: '{}'", node.name))?;
        let kind = model_kind.kind();
        let node_image = config_map.get(&(node.model, kind)).ok_or_else(|| {
            anyhow!(
                "Node config not found for model: {} kind: {kind} (node: {})",
                node.model,
                node.name
            )
        })?;

        // Step 2: Resolve version (use node_image version if not set in manifest)
        let resolved_version = if let Some(v) = &node.version {
            v.clone()
        } else {
            tracing::debug!(
                node = %node.name,
                model = ?node.model,
                version = %node_image.version,
                "Using version from node_image (not set in manifest)"
            );
            node_image.version.clone()
        };

        // Step 3: Validate resolved version exists in database for this model
        validate_version_in_db(model_kind, &resolved_version, node_images).context(format!(
            "Version validation failed for node: {} (model: {})",
            node.name, node.model
        ))?;

        // Step 4: Check image/disk existence based on node kind
        match node_image.kind() {
            NodeKind::Container => {
                let repo = node_images
                    .iter()
                    .find(|c| {
                        c.model == node.model && c.kind() == kind && c.version == resolved_version
                    })
                    .and_then(|c| c.container_repo(node.image.as_deref()));
                updated_node.image = Some(validate_container_repo(
                    &node.name,
                    node.model,
                    &resolved_version,
                    repo,
                )?);
            }
            NodeKind::VirtualMachine | NodeKind::Unikernel => {
                validate_disk_image(
                    &node.name,
                    node.model,
                    &resolved_version,
                    images_dir,
                    &node_image.settings,
                )?;
            }
        }

        // Update node with resolved version
        updated_node.version = Some(resolved_version);
        validated_nodes.push(updated_node);
    }

    Ok(validated_nodes)
}

/// Validate that a version exists in `node_image` for the given model and kind
fn validate_version_in_db(
    model_kind: ModelKind,
    version: &str,
    node_images: &[NodeConfig],
) -> Result<()> {
    let model = model_kind.model();
    let kind = model_kind.kind();
    let of_model_kind = |c: &&NodeConfig| c.model == model && c.kind() == kind;
    let matching_config = node_images
        .iter()
        .filter(of_model_kind)
        .find(|c| c.version == version);

    if matching_config.is_some() {
        Ok(())
    } else {
        // Build helpful error with available versions
        let available_versions: Vec<&str> = node_images
            .iter()
            .filter(of_model_kind)
            .map(|c| c.version.as_str())
            .collect();

        if available_versions.is_empty() {
            bail!("No configurations found in database for model: {model} kind: {kind}");
        }
        bail!(
            "Version '{}' not found in database for model: {} kind: {}. Available versions: {}",
            version,
            model,
            kind,
            available_versions.join(", ")
        );
    }
}

/// The image repo container node `node_name` runs at `version`, as resolved by
/// `NodeConfig::container_repo`; an error naming both fixes when it has none.
pub fn validate_container_repo(
    node_name: &str,
    model: NodeModel,
    version: &str,
    repo: Option<String>,
) -> Result<String> {
    repo.with_context(|| {
        format!(
            "Container node '{node_name}' (model: {model}) has no image repo configured: set \
             `image` on the node in the manifest, or pull the image with `sherpa server image \
             pull --model {model} --kind container --repo <image>:{version}`"
        )
    })
}

/// Validate disk image file exists on filesystem for VMs and unikernels.
fn validate_disk_image(
    node_name: &str,
    model: NodeModel,
    version: &str,
    images_dir: &str,
    settings: &KindSettings,
) -> Result<()> {
    let filename = image_filename(settings).with_context(|| {
        format!("Node '{node_name}' (model: {model}) is a container, which has no disk image")
    })?;
    let disk_path = format!("{images_dir}/{model}/{version}/{filename}");
    let path = Path::new(&disk_path);

    if path.exists() {
        tracing::debug!(
            node = %node_name,
            disk_path = %disk_path,
            "Disk image found on filesystem"
        );
        Ok(())
    } else {
        bail!(
            "Disk image not found for node '{node_name}' (model: {model}, version: {version})\n\
            Expected path: {disk_path}\n\
            Hint: Ensure the image is installed in the correct directory"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::ContainerSettings;

    /// An image of `model` as `kind` at `version`, not the default; a container's repo is
    /// `test-repo`.
    fn create_test_node_image(model: NodeModel, version: &str, kind: NodeKind) -> NodeConfig {
        let mut config = NodeConfig {
            version: version.to_string(),
            default: false,
            ..ModelKind::new(model, Some(kind)).unwrap().node_config()
        };
        set_repo(&mut config, Some("test-repo"));
        config
    }

    /// Set a container image's repo; other kinds have none.
    fn set_repo(config: &mut NodeConfig, repo: Option<&str>) {
        if let KindSettings::Container(container) = &mut config.settings {
            *container = ContainerSettings {
                repo: repo.map(str::to_string),
                ..container.clone()
            };
        }
    }

    #[test]
    fn test_validate_version_in_db_found() {
        let model = NodeModel::AristaEos;
        let configs = vec![
            create_test_node_image(model, "4.28.0F", NodeKind::VirtualMachine),
            create_test_node_image(model, "4.29.2F", NodeKind::VirtualMachine),
        ];

        let result = validate_version_in_db(ModelKind::default_for(model), "4.28.0F", &configs);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_version_in_db_not_found() {
        let model = NodeModel::AristaEos;
        let configs = vec![
            create_test_node_image(model, "4.28.0F", NodeKind::VirtualMachine),
            create_test_node_image(model, "4.29.2F", NodeKind::VirtualMachine),
        ];

        let result = validate_version_in_db(ModelKind::default_for(model), "4.30.0F", &configs);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("4.30.0F"));
        assert!(error_msg.contains("Available versions"));
        assert!(error_msg.contains("4.28.0F"));
        assert!(error_msg.contains("4.29.2F"));
    }

    #[test]
    fn test_validate_version_in_db_no_configs() {
        let model = NodeModel::AristaEos;
        let configs = vec![];

        let result = validate_version_in_db(ModelKind::default_for(model), "4.28.0F", &configs);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("No configurations found"));
    }

    #[test]
    fn test_validate_container_repo_without_a_repo_names_both_fixes() {
        let error =
            validate_container_repo("test-node", NodeModel::AristaEos, "1.0.0", None).unwrap_err();

        let message = error.to_string();
        assert!(message.contains("no image repo configured"), "{message}");
        assert!(message.contains("`image`"), "{message}");
        assert!(
            message.contains("--model arista_eos --kind container --repo <image>:1.0.0"),
            "{message}"
        );
    }

    // ============================================================================
    // validate_disk_image tests
    // ============================================================================

    #[test]
    fn test_validate_disk_image_vm_not_found() {
        let result = validate_disk_image(
            "vm1",
            NodeModel::AristaEos,
            "4.28.0F",
            "/tmp/nonexistent_sherpa_images",
            &ModelKind::default_for(NodeModel::AristaEos)
                .node_config()
                .settings,
        );
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Disk image not found"));
        assert!(error_msg.contains("vm1"));
        assert!(error_msg.contains("virtioa.qcow2"));
    }

    #[test]
    fn test_validate_disk_image_vm_path_format() {
        let result = validate_disk_image(
            "vm1",
            NodeModel::RockyLinux,
            "9.3",
            "/opt/images",
            &ModelKind::default_for(NodeModel::RockyLinux)
                .node_config()
                .settings,
        );
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("/opt/images/rocky_linux/9.3/virtioa.qcow2"));
    }

    #[test]
    fn test_validate_disk_image_unikernel_direct_kernel() {
        let result = validate_disk_image(
            "uk1",
            NodeModel::UnikraftUnikernel,
            "latest",
            "/opt/images",
            &ModelKind::default_for(NodeModel::UnikraftUnikernel)
                .node_config()
                .settings,
        );
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("kernel.elf"));
    }

    #[test]
    fn test_validate_disk_image_unikernel_disk_boot() {
        let result = validate_disk_image(
            "uk1",
            NodeModel::NanosUnikernel,
            "latest",
            "/opt/images",
            &ModelKind::default_for(NodeModel::NanosUnikernel)
                .node_config()
                .settings,
        );
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("disk.qcow2"));
    }

    // ============================================================================
    // validate_and_resolve_node_versions tests (public entry point)
    // ============================================================================

    fn create_test_node(name: &str, model: NodeModel, version: Option<&str>) -> Node {
        Node {
            name: name.to_string(),
            model,
            version: version.map(std::string::ToString::to_string),
            ..Default::default()
        }
    }

    /// A container node of `model`, which may also take another kind.
    fn create_test_container(name: &str, model: NodeModel, version: Option<&str>) -> Node {
        Node {
            kind: Some(NodeKind::Container),
            ..create_test_node(name, model, version)
        }
    }

    #[test]
    fn test_resolve_explicit_version_takes_precedence() {
        let nodes = vec![create_test_container(
            "ceos1",
            NodeModel::AristaEos,
            Some("4.31.0F"),
        )];
        let mut default_config =
            create_test_node_image(NodeModel::AristaEos, "4.32.0F", NodeKind::Container);
        default_config.default = true;
        let explicit_config =
            create_test_node_image(NodeModel::AristaEos, "4.31.0F", NodeKind::Container);

        let result = validate_and_resolve_node_versions(
            &nodes,
            &[default_config, explicit_config],
            "/tmp/images",
        );

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved[0].version, Some("4.31.0F".to_string()));
    }

    #[test]
    fn test_resolve_prefers_default_node_image() {
        let nodes = vec![create_test_container("ceos1", NodeModel::AristaEos, None)];
        let non_default =
            create_test_node_image(NodeModel::AristaEos, "4.30.0F", NodeKind::Container);
        let mut default =
            create_test_node_image(NodeModel::AristaEos, "4.32.0F", NodeKind::Container);
        default.default = true;

        let result =
            validate_and_resolve_node_versions(&nodes, &[non_default, default], "/tmp/images");

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved[0].version, Some("4.32.0F".to_string()));
    }

    #[test]
    fn test_resolve_no_config_for_model_fails() {
        let nodes = vec![create_test_node("vm1", NodeModel::AristaEos, None)];
        // Provide only an image of the model's other kind
        let config = create_test_node_image(NodeModel::AristaEos, "4.32.0F", NodeKind::Container);

        let result = validate_and_resolve_node_versions(&nodes, &[config], "/tmp/images");

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Node config not found"));
        assert!(error_msg.contains("vm1"));
    }

    #[test]
    fn test_resolve_version_not_in_db_fails() {
        let nodes = vec![create_test_container(
            "ceos1",
            NodeModel::AristaEos,
            Some("9.99.0F"),
        )];
        let config = create_test_node_image(NodeModel::AristaEos, "4.32.0F", NodeKind::Container);

        let result = validate_and_resolve_node_versions(&nodes, &[config], "/tmp/images");

        assert!(result.is_err());
        let error_msg = format!("{:#}", result.unwrap_err());
        assert!(error_msg.contains("9.99.0F"));
    }

    fn generic_image(version: &str, repo: Option<&str>, default: bool) -> NodeConfig {
        let mut config = create_test_node_image(NodeModel::Generic, version, NodeKind::Container);
        set_repo(&mut config, repo);
        config.default = default;
        config
    }

    #[test]
    fn test_resolve_container_without_built_in_repo_uses_manifest_image() {
        let mut node = create_test_container("host1", NodeModel::Generic, Some("24.04"));
        node.image = Some("ubuntu".to_string());
        let images = [
            generic_image("3.22", None, true),
            generic_image("24.04", None, false),
        ];

        let resolved = validate_and_resolve_node_versions(&[node], &images, "/tmp/images").unwrap();

        assert_eq!(resolved[0].image.as_deref(), Some("ubuntu"));
    }

    #[test]
    fn test_resolve_container_without_built_in_repo_uses_repo_pulled_for_its_version() {
        // The model's default image is another repo; the node's version decides.
        let nodes = [create_test_container(
            "host1",
            NodeModel::Generic,
            Some("24.04"),
        )];
        let images = [
            generic_image("3.22", Some("alpine"), true),
            generic_image("24.04", Some("ubuntu"), false),
        ];

        let resolved = validate_and_resolve_node_versions(&nodes, &images, "/tmp/images").unwrap();

        assert_eq!(resolved[0].image.as_deref(), Some("ubuntu"));
    }

    #[test]
    fn test_resolve_container_without_image_or_repo_fails_naming_both_fixes() {
        let nodes = [create_test_node("db1", NodeModel::PostgresqlDb, Some("17"))];
        let mut config = create_test_node_image(NodeModel::PostgresqlDb, "17", NodeKind::Container);
        set_repo(&mut config, None);
        config.default = true;

        let error =
            validate_and_resolve_node_versions(&nodes, &[config], "/tmp/images").unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("`image`"), "{message}");
        assert!(message.contains("--repo"), "{message}");
    }

    /// The image a container node of `model` resolves to when its version was pulled from
    /// `repo`.
    fn resolved_image(model: NodeModel, repo: &str, manifest_image: Option<&str>) -> String {
        let mut node = create_test_node("node1", model, Some("2.11"));
        node.image = manifest_image.map(str::to_string);
        let mut config = create_test_node_image(model, "2.11", NodeKind::Container);
        set_repo(&mut config, Some(repo));
        config.default = true;

        let resolved = validate_and_resolve_node_versions(&[node], &[config], "/tmp/images")
            .unwrap()
            .remove(0);

        assert_eq!(resolved.version.as_deref(), Some("2.11"));
        resolved.image.unwrap()
    }

    #[test]
    fn test_resolve_container_keeps_an_official_image_pulled_by_its_full_name() {
        assert_eq!(
            resolved_image(NodeModel::NatsBus, "docker.io/library/nats", None),
            "docker.io/library/nats"
        );
    }

    #[test]
    fn test_resolve_container_keeps_an_official_image_pulled_by_its_short_name() {
        assert_eq!(resolved_image(NodeModel::NatsBus, "nats", None), "nats");
    }

    #[test]
    fn test_resolve_container_keeps_a_third_party_registry_path() {
        assert_eq!(
            resolved_image(
                NodeModel::NokiaSrlinux,
                "registry.example:5000/mirror/srlinux",
                None
            ),
            "registry.example:5000/mirror/srlinux"
        );
    }

    #[test]
    fn test_resolve_container_with_a_built_in_repo_runs_the_manifest_image() {
        assert_eq!(
            resolved_image(NodeModel::NatsBus, "nats", Some("mirror.example/nats")),
            "mirror.example/nats"
        );
    }

    #[test]
    fn test_resolve_takes_the_image_of_the_nodes_kind_only() {
        // arista_eos takes two kinds; an image row of the other kind is not the node's.
        let nodes = vec![create_test_container("ceos1", NodeModel::AristaEos, None)];
        let mut other_kind =
            create_test_node_image(NodeModel::AristaEos, "9.9.9", NodeKind::VirtualMachine);
        other_kind.default = true;
        let mut own_kind =
            create_test_node_image(NodeModel::AristaEos, "4.32.0F", NodeKind::Container);
        own_kind.default = true;
        let resolved =
            validate_and_resolve_node_versions(&nodes, &[other_kind.clone(), own_kind], "/tmp")
                .unwrap();
        assert_eq!(resolved[0].version.as_deref(), Some("4.32.0F"));

        let nodes = vec![create_test_container(
            "ceos1",
            NodeModel::AristaEos,
            Some("9.9.9"),
        )];
        let error = validate_and_resolve_node_versions(&nodes, &[other_kind], "/tmp")
            .unwrap_err()
            .to_string();
        assert!(error.contains("kind: container"), "{error}");
    }

    #[test]
    fn test_resolve_refuses_a_kind_the_model_does_not_take() {
        let mut node = create_test_node("iol1", NodeModel::CiscoIol, None);
        node.kind = Some(NodeKind::VirtualMachine);
        let mut config = create_test_node_image(NodeModel::CiscoIol, "17.12", NodeKind::Container);
        config.default = true;
        let error = validate_and_resolve_node_versions(&[node], &[config], "/tmp").unwrap_err();
        assert!(
            format!("{error:#}").contains("comes only as: container"),
            "{error:#}"
        );
    }
}
