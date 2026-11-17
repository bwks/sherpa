use anyhow::Result;
use tracing::instrument;

/// Process manifest nodes into expanded format with indices assigned, each with its kind.
///
/// # Errors
///
/// Fails when a node sets a `kind` its model does not take.
#[instrument(level = "debug", skip(manifest_nodes))]
pub fn process_manifest_nodes(
    manifest_nodes: &[topology::Node],
) -> Result<Vec<topology::NodeExpanded>> {
    // Node indexes start from 1, which aligns with IP address assignment.
    // check_device_count caps a lab far below u16::MAX nodes.
    (1..=u16::MAX)
        .zip(manifest_nodes)
        .map(|(index, node)| topology::NodeExpanded::new(index, node))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{self, ModelKind, NodeModel};

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
                model: data::NodeModel::Generic,
                kind: Some(data::NodeKind::Container),
                ..Default::default()
            },
            topology::Node {
                name: "unikernel".into(),
                model: data::NodeModel::Generic,
                kind: Some(data::NodeKind::Unikernel),
                ..Default::default()
            },
            topology::Node {
                name: "unsupported-overlay".into(),
                model: data::NodeModel::Generic,
                kind: Some(data::NodeKind::Container),
                boot_disk: data::NodeBootDisk {
                    clone_method: Some(data::DiskImageCloneMethod::Overlay),
                    ..Default::default()
                },
                ..Default::default()
            },
        ];
        let expanded = process_manifest_nodes(&nodes).unwrap();
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

        let expanded = process_manifest_nodes(&nodes).unwrap();

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
            model: NodeModel::AristaEos,
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

        let expanded = process_manifest_nodes(&nodes).unwrap();

        assert_eq!(
            expanded[0].model_kind,
            ModelKind::default_for(NodeModel::AristaEos)
        );
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
        let expanded = process_manifest_nodes(&nodes).unwrap();
        assert_eq!(expanded, [] as [topology::NodeExpanded; 0]);
    }
}
