use anyhow::{Result, anyhow};
use tracing::instrument;

use shared::data;

/// Process manifest nodes into expanded format with indices assigned
#[instrument(level = "debug", skip(manifest_nodes))]
pub fn process_manifest_nodes(manifest_nodes: &[topology::Node]) -> Vec<topology::NodeExpanded> {
    // Node indexes start from 1, which aligns with IP address assignment.
    // check_device_count caps a lab far below u16::MAX nodes.
    (1..=u16::MAX)
        .zip(manifest_nodes)
        .map(|(index, node)| topology::NodeExpanded::new(index, node))
        .collect()
}

/// Get node image from a list of node images
pub fn get_node_image(
    node_model: data::NodeModel,
    data: &[data::NodeConfig],
) -> Result<data::NodeConfig> {
    Ok(data
        .iter()
        .find(|x| x.model == node_model && x.default)
        .ok_or_else(|| anyhow!("Default node image not found for model: {node_model}"))?
        .clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeModel;

    // ============================================================================

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
                boot_disk: data::NodeBootDisk {
                    clone_method: Some(data::DiskImageCloneMethod::Copy),
                    ..Default::default()
                },
                ..Default::default()
            },
            topology::Node {
                name: "overlay-vm".into(),
                model: data::NodeModel::UbuntuLinux,
                boot_disk: data::NodeBootDisk {
                    clone_method: Some(data::DiskImageCloneMethod::Overlay),
                    ..Default::default()
                },
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
                boot_disk: data::NodeBootDisk {
                    clone_method: Some(data::DiskImageCloneMethod::Overlay),
                    ..Default::default()
                },
                ..Default::default()
            },
        ];
        let expanded = process_manifest_nodes(&nodes);
        assert_eq!(expanded[0].boot_disk.clone_method, None);
        assert_eq!(
            expanded[1].boot_disk.clone_method,
            Some(data::DiskImageCloneMethod::Copy)
        );
        assert_eq!(
            expanded[2].boot_disk.clone_method,
            Some(data::DiskImageCloneMethod::Overlay)
        );
        assert_eq!(expanded[3].boot_disk.clone_method, None);
        assert_eq!(expanded[4].boot_disk.clone_method, None);
        // Preserve an explicit unsupported request so validation rejects it.
        assert_eq!(
            expanded[5].boot_disk.clone_method,
            Some(data::DiskImageCloneMethod::Overlay)
        );
    }
    // process_manifest_nodes
    // ============================================================================

    #[test]
    fn test_process_manifest_nodes_assigns_indices_starting_at_one() {
        let nodes = vec![
            topology::Node {
                name: "dev01".to_string(),
                model: NodeModel::UbuntuLinux,
                ..Default::default()
            },
            topology::Node {
                name: "dev02".to_string(),
                model: NodeModel::UbuntuLinux,
                ..Default::default()
            },
            topology::Node {
                name: "dev03".to_string(),
                model: NodeModel::UbuntuLinux,
                ..Default::default()
            },
        ];

        let expanded = process_manifest_nodes(&nodes);

        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0].index, 1);
        assert_eq!(expanded[0].name, "dev01");
        assert_eq!(expanded[1].index, 2);
        assert_eq!(expanded[1].name, "dev02");
        assert_eq!(expanded[2].index, 3);
        assert_eq!(expanded[2].name, "dev03");
    }

    #[test]
    fn test_process_manifest_nodes_preserves_fields() {
        let nodes = vec![topology::Node {
            name: "router01".to_string(),
            model: NodeModel::AristaVeos,
            version: Some("4.28.0".to_string()),
            memory: Some(4096),
            cpu: data::NodeCpu {
                count: Some(2),
                ..Default::default()
            },
            interfaces: data::NodeInterfaces {
                data_count: Some(4),
                queues: None,
            },
            vm: Some(topology::NodeVm::default()),
            ready_check: topology::NodeReadyCheck {
                port: None,
                skip: Some(true),
            },
            ..Default::default()
        }];

        let expanded = process_manifest_nodes(&nodes);

        assert_eq!(expanded[0].model, NodeModel::AristaVeos);
        assert_eq!(expanded[0].version, Some("4.28.0".to_string()));
        assert_eq!(expanded[0].memory, Some(4096));
        assert_eq!(expanded[0].cpu.count, Some(2));
        assert_eq!(expanded[0].interfaces.data_count, Some(4));
        assert!(expanded[0].vm.is_some());
        assert_eq!(expanded[0].ready_check.skip, Some(true));
    }

    #[test]
    fn test_process_manifest_nodes_empty_input() {
        let nodes: Vec<topology::Node> = vec![];
        let expanded = process_manifest_nodes(&nodes);
        assert!(expanded.is_empty());
    }

    // ============================================================================
    // get_node_image
    // ============================================================================

    #[test]
    fn test_get_node_image_finds_default() {
        let configs = vec![
            data::NodeConfig {
                model: NodeModel::UbuntuLinux,
                version: "22.04".to_string(),
                default: false,
                ..Default::default()
            },
            data::NodeConfig {
                model: NodeModel::UbuntuLinux,
                version: "24.04".to_string(),
                default: true,
                ..Default::default()
            },
        ];

        let result = get_node_image(NodeModel::UbuntuLinux, &configs).unwrap();
        assert_eq!(result.version, "24.04");
        assert!(result.default);
    }

    #[test]
    fn test_get_node_image_no_default_returns_error() {
        let configs = vec![data::NodeConfig {
            model: NodeModel::UbuntuLinux,
            version: "22.04".to_string(),
            default: false,
            ..Default::default()
        }];

        let result = get_node_image(NodeModel::UbuntuLinux, &configs);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("Default node image not found"));
    }

    #[test]
    fn test_get_node_image_wrong_model_returns_error() {
        let configs = vec![data::NodeConfig {
            model: NodeModel::AristaVeos,
            version: "4.28.0".to_string(),
            default: true,
            ..Default::default()
        }];

        let result = get_node_image(NodeModel::UbuntuLinux, &configs);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_node_image_empty_configs_returns_error() {
        let configs: Vec<data::NodeConfig> = vec![];
        let result = get_node_image(NodeModel::UbuntuLinux, &configs);
        assert!(result.is_err());
    }
}
