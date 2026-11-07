//! Expanding manifest nodes and finding their images.

use anyhow::{Result, anyhow};
use tracing::instrument;

use shared::data;

/// Process manifest nodes into expanded format with indices assigned
#[instrument(level = "debug", skip(manifest_nodes))]
pub(crate) fn process_manifest_nodes(
    manifest_nodes: &[topology::Node],
) -> Vec<topology::NodeExpanded> {
    // check_device_count caps a lab far below u16::MAX nodes.
    (1..=u16::MAX)
        .zip(manifest_nodes)
        .map(|(index, node)| topology::NodeExpanded {
            boot_disk_size: node.boot_disk_size.or(node.model.default_boot_disk_size()),
            ..topology::NodeExpanded::new(index, node)
        })
        .collect()
}

/// Get node image from a list of node images.
/// When a version is provided, match on that version. Otherwise fall back to the default.
pub(super) fn get_node_image(
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

#[cfg(test)]
mod tests {
    use super::*;

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
