//! Expanding manifest nodes and finding their images.

use anyhow::{Result, anyhow};
use tracing::instrument;

use shared::data;

/// Process manifest nodes into expanded format with indices assigned, each with its kind.
///
/// # Errors
///
/// Fails when a node sets a `kind` its model does not take.
#[instrument(level = "debug", skip(manifest_nodes))]
pub(crate) fn process_manifest_nodes(
    manifest_nodes: &[topology::Node],
) -> Result<Vec<topology::NodeExpanded>> {
    // check_device_count caps a lab far below u16::MAX nodes.
    (1..=u16::MAX)
        .zip(manifest_nodes)
        .map(|(index, node)| {
            let mut expanded = topology::NodeExpanded::new(index, node)?;
            expanded.boot_disk.size = node
                .boot_disk
                .size
                .or(expanded.model_kind.default_boot_disk_size());
            Ok(expanded)
        })
        .collect()
}

/// Get node image from a list of node images: the image of the node's model and kind.
/// When a version is provided, match on that version. Otherwise fall back to the default.
pub(super) fn get_node_image(
    model_kind: data::ModelKind,
    version: Option<&str>,
    data: &[data::NodeConfig],
) -> Result<data::NodeConfig> {
    let model = model_kind.model();
    let kind = model_kind.kind();
    let mut of_model_kind = data.iter().filter(|x| x.model == model && x.kind() == kind);
    if let Some(v) = version {
        of_model_kind
            .find(|x| x.version == v)
            .cloned()
            .ok_or_else(|| {
                anyhow!("Node image not found for model: {model} kind: {kind} version: {v}")
            })
    } else {
        of_model_kind
            .find(|x| x.default)
            .cloned()
            .ok_or_else(|| anyhow!("Default node image not found for model: {model} kind: {kind}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::validate_and_resolve_node_versions;

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
                boot_disk: data::NodeBootDisk {
                    size: Some(50),
                    ..Default::default()
                },
                ..Default::default()
            },
            topology::Node {
                name: "ubuntu".into(),
                model: data::NodeModel::UbuntuLinux,
                ..Default::default()
            },
        ])
        .unwrap();
        assert_eq!(expanded[0].boot_disk.size, Some(20));
        assert_eq!(expanded[1].boot_disk.size, Some(50));
        assert_eq!(expanded[2].boot_disk.size, None);
    }

    #[test]
    fn vm_start_011_clone_method_reaches_each_vm_node_with_copy_overlay_and_default_kept_apart() {
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
    }

    /// An `arista_eos` image of `kind`, which the model takes both of.
    fn image(kind: data::NodeKind, version: &str, default: bool) -> data::NodeConfig {
        data::NodeConfig {
            version: version.to_owned(),
            default,
            ..data::ModelKind::new(data::NodeModel::AristaEos, Some(kind))
                .unwrap()
                .node_config()
        }
    }

    #[test]
    fn image_selection_003_a_node_gets_an_image_of_its_own_kind_only() {
        let sonic = |kind| data::ModelKind::new(data::NodeModel::SonicLinux, Some(kind)).unwrap();
        let images = [
            data::NodeConfig {
                version: "202511".to_owned(),
                default: true,
                ..sonic(data::NodeKind::VirtualMachine).node_config()
            },
            data::NodeConfig {
                version: "202511".to_owned(),
                default: true,
                ..sonic(data::NodeKind::Container).node_config()
            },
        ];
        for kind in [data::NodeKind::VirtualMachine, data::NodeKind::Container] {
            for version in [None, Some("202511")] {
                let found = get_node_image(sonic(kind), version, &images).unwrap();
                assert_eq!(found.kind(), kind, "{version:?}");
                assert_eq!(found.version, "202511");
            }
        }

        // An image row of another kind is never the node's, default or not.
        let mut own_kind = image(data::NodeKind::Container, "10.5", true);
        if let data::KindSettings::Container(container) = &mut own_kind.settings {
            container.repo = Some("test-repo".to_owned());
        }
        let images = [
            image(data::NodeKind::VirtualMachine, "9.9", true),
            own_kind,
            image(data::NodeKind::Container, "10.4", false),
        ];
        let ceos =
            data::ModelKind::new(data::NodeModel::AristaEos, Some(data::NodeKind::Container))
                .unwrap();
        assert_eq!(get_node_image(ceos, None, &images).unwrap().version, "10.5");
        assert_eq!(
            get_node_image(ceos, Some("10.4"), &images).unwrap().version,
            "10.4"
        );
        let error = get_node_image(ceos, Some("9.9"), &images)
            .unwrap_err()
            .to_string();
        assert!(error.contains("kind: container"), "{error}");

        // Up's check of the manifest takes the same image, and refuses the other kind's version.
        let node = |version: Option<&str>| topology::Node {
            name: "ceos1".into(),
            model: data::NodeModel::AristaEos,
            kind: Some(data::NodeKind::Container),
            version: version.map(str::to_owned),
            ..Default::default()
        };
        let resolved = validate_and_resolve_node_versions(&[node(None)], &images, "/tmp").unwrap();
        assert_eq!(resolved[0].version.as_deref(), Some("10.5"));
        let error =
            validate_and_resolve_node_versions(&[node(Some("9.9"))], &images, "/tmp").unwrap_err();
        assert!(
            format!("{error:#}").contains("kind: container"),
            "{error:#}"
        );
    }
}
