use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashMap;
use std::path::Path;

use shared::data::{NodeConfig, NodeKind, NodeModel};
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

    // Build lookup map for faster access
    let config_map: HashMap<NodeModel, &NodeConfig> =
        node_images.iter().map(|c| (c.model, c)).collect();

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
            NodeKind::VirtualMachine => {
                validate_vm_disk(&node.name, &node.model, &resolved_version, images_dir)?;
            }
            NodeKind::Unikernel => {
                // Unikernels use same disk structure as VMs
                validate_vm_disk(&node.name, &node.model, &resolved_version, images_dir)?;
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

/// Validate VM disk file exists on filesystem
fn validate_vm_disk(
    node_name: &str,
    model: &NodeModel,
    version: &str,
    images_dir: &str,
) -> Result<()> {
    let disk_path = format!("{}/{}/{}/virtioa.qcow2", images_dir, model, version);
    let path = Path::new(&disk_path);

    if path.exists() {
        tracing::debug!(
            node = %node_name,
            disk_path = %disk_path,
            "VM disk image found on filesystem"
        );
        Ok(())
    } else {
        bail!(
            "VM disk image not found for node '{}' (model: {}, version: {})\n\
            Expected path: {}\n\
            Hint: Ensure the VM image is installed in the correct directory",
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
}
