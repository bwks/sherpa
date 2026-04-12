use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashMap;
use std::path::Path;

use shared::data::{NodeConfig, NodeKind, NodeModel, UnikernelBootMode};
use shared::util::image_filename;
use topology::Node;

/// Validate and resolve node versions, checking that images exist
///
/// This function performs three key validations:
/// 1. Resolves version from node_image if not set in manifest
/// 2. Validates version exists in database for the model
/// 3. Verifies image/disk exists based on node kind
///
/// # Arguments
/// * `nodes` - List of nodes from manifest
/// * `node_images` - Pre-fetched list of all node configs from DB
/// * `images_dir` - Base directory for VM images
/// * `docker_images` - List of local Docker images (format: "repo:tag")
///
/// # Returns
/// Updated nodes with version field populated
///
/// # Errors
/// Returns error if:
/// - Version not found in node_image for model
/// - Container image not found in local Docker
/// - VM disk file not found on filesystem
pub fn validate_and_resolve_node_versions(
    nodes: &[Node],
    node_images: &[NodeConfig],
    images_dir: &str,
    docker_images: &[String],
) -> Result<Vec<Node>> {
    let mut validated_nodes = Vec::new();

    // Build lookup map for faster access, preferring default images per model
    let mut config_map: HashMap<NodeModel, &NodeConfig> = HashMap::new();
    for config in node_images {
        config_map
            .entry(config.model)
            .and_modify(|existing| {
                if config.default && !existing.default {
                    *existing = config;
                }
            })
            .or_insert(config);
    }

    for node in nodes {
        let mut updated_node = node.clone();

        // Step 1: Get node_image for this model
        let node_image = config_map.get(&node.model).ok_or_else(|| {
            anyhow!(
                "Node config not found for model: {} (node: {})",
                node.model,
                node.name
            )
        })?;

        // Step 2: Resolve version (use node_image version if not set in manifest)
        let resolved_version = match &node.version {
            Some(v) => v.clone(),
            None => {
                tracing::debug!(
                    node = %node.name,
                    model = ?node.model,
                    version = %node_image.version,
                    "Using version from node_image (not set in manifest)"
                );
                node_image.version.clone()
            }
        };

        // Step 3: Validate resolved version exists in database for this model
        validate_version_in_db(&node.model, &resolved_version, node_images).context(format!(
            "Version validation failed for node: {} (model: {})",
            node.name, node.model
        ))?;

        // Step 4: Check image/disk existence based on node kind
        match node_image.kind {
            NodeKind::Container => {
                validate_container_image(
                    &node.name,
                    &node.model,
                    node_image.repo.as_deref(),
                    &resolved_version,
                    docker_images,
                )?;
            }
            NodeKind::VirtualMachine | NodeKind::Unikernel => {
                validate_disk_image(
                    &node.name,
                    &node.model,
                    &resolved_version,
                    images_dir,
                    &node_image.kind,
                    node_image.boot_mode.as_ref(),
                )?;
            }
        }

        // Update node with resolved version
        updated_node.version = Some(resolved_version);
        validated_nodes.push(updated_node);
    }

    Ok(validated_nodes)
}

/// Validate that a version exists in node_image for the given model
fn validate_version_in_db(
    model: &NodeModel,
    version: &str,
    node_images: &[NodeConfig],
) -> Result<()> {
    let matching_config = node_images
        .iter()
        .find(|c| c.model == *model && c.version == version);

    match matching_config {
        Some(_) => Ok(()),
        None => {
            // Build helpful error with available versions
            let available_versions: Vec<&str> = node_images
                .iter()
                .filter(|c| c.model == *model)
                .map(|c| c.version.as_str())
                .collect();

            if available_versions.is_empty() {
                bail!("No configurations found in database for model: {}", model);
            } else {
                bail!(
                    "Version '{}' not found in database for model: {}. Available versions: {}",
                    version,
                    model,
                    available_versions.join(", ")
                );
            }
        }
    }
}

/// Validate container image exists in local Docker
fn validate_container_image(
    node_name: &str,
    model: &NodeModel,
    repo: Option<&str>,
    version: &str,
    docker_images: &[String],
) -> Result<()> {
    // Build expected image name
    let expected_image = match repo {
        Some(r) => format!("{}:{}", r, version),
        None => {
            bail!(
                "Container node '{}' (model: {}) has no repo configured in node_image",
                node_name,
                model
            );
        }
    };

    // Check if image exists locally
    if docker_images.contains(&expected_image) {
        tracing::debug!(
            node = %node_name,
            image = %expected_image,
            "Container image found in local Docker"
        );
        Ok(())
    } else {
        bail!(
            "Container image not found in local Docker for node '{}': {}\n\
            Hint: Pull the image with: docker pull {}",
            node_name,
            expected_image,
            expected_image
        );
    }
}

/// Validate disk image file exists on filesystem for VMs and unikernels.
fn validate_disk_image(
    node_name: &str,
    model: &NodeModel,
    version: &str,
    images_dir: &str,
    kind: &NodeKind,
    boot_mode: Option<&UnikernelBootMode>,
) -> Result<()> {
    let filename = image_filename(kind, boot_mode);
    let disk_path = format!("{}/{}/{}/{}", images_dir, model, version, filename);
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
            "Disk image not found for node '{}' (model: {}, version: {})\n\
            Expected path: {}\n\
            Hint: Ensure the image is installed in the correct directory",
            node_name,
            model,
            version,
            disk_path
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{
        BiosTypes, CpuArchitecture, CpuModels, DiskBuses, InterfaceType, MachineType,
        MgmtInterfaces, OsVariant, ZtpMethod,
    };

    fn create_test_node_image(model: NodeModel, version: &str, kind: NodeKind) -> NodeConfig {
        NodeConfig {
            id: None,
            model,
            version: version.to_string(),
            repo: Some("test-repo".to_string()),
            os_variant: OsVariant::Linux,
            kind,
            bios: BiosTypes::SeaBios,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: false,
            ztp_method: ZtpMethod::None,
            ztp_username: None,
            ztp_password: None,
            ztp_password_auth: false,
            data_interface_count: 4,
            interface_prefix: "eth".to_string(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: 1500,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
            default: false,
            boot_mode: None,
        }
    }

    #[test]
    fn test_validate_version_in_db_found() {
        let model = NodeModel::AristaVeos;
        let configs = vec![
            create_test_node_image(model.clone(), "4.28.0F", NodeKind::VirtualMachine),
            create_test_node_image(model.clone(), "4.29.2F", NodeKind::VirtualMachine),
        ];

        let result = validate_version_in_db(&model, "4.28.0F", &configs);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_version_in_db_not_found() {
        let model = NodeModel::AristaVeos;
        let configs = vec![
            create_test_node_image(model.clone(), "4.28.0F", NodeKind::VirtualMachine),
            create_test_node_image(model.clone(), "4.29.2F", NodeKind::VirtualMachine),
        ];

        let result = validate_version_in_db(&model, "4.30.0F", &configs);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("4.30.0F"));
        assert!(error_msg.contains("Available versions"));
        assert!(error_msg.contains("4.28.0F"));
        assert!(error_msg.contains("4.29.2F"));
    }

    #[test]
    fn test_validate_version_in_db_no_configs() {
        let model = NodeModel::AristaVeos;
        let configs = vec![];

        let result = validate_version_in_db(&model, "4.28.0F", &configs);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("No configurations found"));
    }

    #[test]
    fn test_validate_container_image_found() {
        let docker_images = vec![
            "test-repo:1.0.0".to_string(),
            "test-repo:2.0.0".to_string(),
            "other-repo:1.0.0".to_string(),
        ];

        let result = validate_container_image(
            "test-node",
            &NodeModel::AristaCeos,
            Some("test-repo"),
            "1.0.0",
            &docker_images,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_container_image_not_found() {
        let docker_images = vec![
            "test-repo:1.0.0".to_string(),
            "other-repo:1.0.0".to_string(),
        ];

        let result = validate_container_image(
            "test-node",
            &NodeModel::AristaCeos,
            Some("test-repo"),
            "2.0.0",
            &docker_images,
        );
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("not found in local Docker"));
        assert!(error_msg.contains("docker pull"));
    }

    #[test]
    fn test_validate_container_image_no_repo() {
        let docker_images = vec![];

        let result = validate_container_image(
            "test-node",
            &NodeModel::AristaCeos,
            None,
            "1.0.0",
            &docker_images,
        );
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("no repo configured"));
    }

    // ============================================================================
    // validate_disk_image tests
    // ============================================================================

    #[test]
    fn test_validate_disk_image_vm_not_found() {
        let result = validate_disk_image(
            "vm1",
            &NodeModel::AristaVeos,
            "4.28.0F",
            "/tmp/nonexistent_sherpa_images",
            &NodeKind::VirtualMachine,
            None,
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
            &NodeModel::RockyLinux,
            "9.3",
            "/opt/images",
            &NodeKind::VirtualMachine,
            None,
        );
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("/opt/images/rocky_linux/9.3/virtioa.qcow2"));
    }

    #[test]
    fn test_validate_disk_image_unikernel_direct_kernel() {
        let result = validate_disk_image(
            "uk1",
            &NodeModel::UnikraftUnikernel,
            "latest",
            "/opt/images",
            &NodeKind::Unikernel,
            Some(&UnikernelBootMode::DirectKernel),
        );
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("kernel.elf"));
    }

    #[test]
    fn test_validate_disk_image_unikernel_disk_boot() {
        let result = validate_disk_image(
            "uk1",
            &NodeModel::NanosUnikernel,
            "latest",
            "/opt/images",
            &NodeKind::Unikernel,
            Some(&UnikernelBootMode::DiskBoot),
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
            version: version.map(|v| v.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_resolve_version_from_node_image_when_not_set() {
        let nodes = vec![create_test_node("ceos1", NodeModel::AristaCeos, None)];
        let mut config =
            create_test_node_image(NodeModel::AristaCeos, "4.32.0F", NodeKind::Container);
        config.default = true;

        let docker_images = vec!["test-repo:4.32.0F".to_string()];
        let result =
            validate_and_resolve_node_versions(&nodes, &[config], "/tmp/images", &docker_images);

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved[0].version, Some("4.32.0F".to_string()));
    }

    #[test]
    fn test_resolve_explicit_version_takes_precedence() {
        let nodes = vec![create_test_node(
            "ceos1",
            NodeModel::AristaCeos,
            Some("4.31.0F"),
        )];
        let mut default_config =
            create_test_node_image(NodeModel::AristaCeos, "4.32.0F", NodeKind::Container);
        default_config.default = true;
        let explicit_config =
            create_test_node_image(NodeModel::AristaCeos, "4.31.0F", NodeKind::Container);

        let docker_images = vec![
            "test-repo:4.32.0F".to_string(),
            "test-repo:4.31.0F".to_string(),
        ];
        let result = validate_and_resolve_node_versions(
            &nodes,
            &[default_config, explicit_config],
            "/tmp/images",
            &docker_images,
        );

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved[0].version, Some("4.31.0F".to_string()));
    }

    #[test]
    fn test_resolve_prefers_default_node_image() {
        let nodes = vec![create_test_node("ceos1", NodeModel::AristaCeos, None)];
        let non_default =
            create_test_node_image(NodeModel::AristaCeos, "4.30.0F", NodeKind::Container);
        let mut default =
            create_test_node_image(NodeModel::AristaCeos, "4.32.0F", NodeKind::Container);
        default.default = true;

        let docker_images = vec![
            "test-repo:4.30.0F".to_string(),
            "test-repo:4.32.0F".to_string(),
        ];
        let result = validate_and_resolve_node_versions(
            &nodes,
            &[non_default, default],
            "/tmp/images",
            &docker_images,
        );

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved[0].version, Some("4.32.0F".to_string()));
    }

    #[test]
    fn test_resolve_no_config_for_model_fails() {
        let nodes = vec![create_test_node("vm1", NodeModel::AristaVeos, None)];
        // Provide configs for a different model
        let config = create_test_node_image(NodeModel::AristaCeos, "4.32.0F", NodeKind::Container);

        let result = validate_and_resolve_node_versions(&nodes, &[config], "/tmp/images", &[]);

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Node config not found"));
        assert!(error_msg.contains("vm1"));
    }

    #[test]
    fn test_resolve_version_not_in_db_fails() {
        let nodes = vec![create_test_node(
            "ceos1",
            NodeModel::AristaCeos,
            Some("9.99.0F"),
        )];
        let config = create_test_node_image(NodeModel::AristaCeos, "4.32.0F", NodeKind::Container);

        let docker_images = vec!["test-repo:4.32.0F".to_string()];
        let result =
            validate_and_resolve_node_versions(&nodes, &[config], "/tmp/images", &docker_images);

        assert!(result.is_err());
        let error_msg = format!("{:#}", result.unwrap_err());
        assert!(error_msg.contains("9.99.0F"));
    }

    #[test]
    fn test_resolve_container_image_missing_from_docker_fails() {
        let nodes = vec![create_test_node("ceos1", NodeModel::AristaCeos, None)];
        let mut config =
            create_test_node_image(NodeModel::AristaCeos, "4.32.0F", NodeKind::Container);
        config.default = true;

        // Empty docker images list
        let result = validate_and_resolve_node_versions(&nodes, &[config], "/tmp/images", &[]);

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("not found in local Docker"));
    }

    #[test]
    fn test_resolve_multiple_nodes_mixed_types() {
        let nodes = vec![
            create_test_node("ceos1", NodeModel::AristaCeos, None),
            create_test_node("ceos2", NodeModel::AristaCeos, Some("4.31.0F")),
        ];
        let mut default =
            create_test_node_image(NodeModel::AristaCeos, "4.32.0F", NodeKind::Container);
        default.default = true;
        let other = create_test_node_image(NodeModel::AristaCeos, "4.31.0F", NodeKind::Container);

        let docker_images = vec![
            "test-repo:4.32.0F".to_string(),
            "test-repo:4.31.0F".to_string(),
        ];
        let result = validate_and_resolve_node_versions(
            &nodes,
            &[default, other],
            "/tmp/images",
            &docker_images,
        );

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved[0].version, Some("4.32.0F".to_string()));
        assert_eq!(resolved[1].version, Some("4.31.0F".to_string()));
    }
}
