use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::ModelKind;
use topology::validate::check_node_file_settings;

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

/// Check each node's `text_files`, `startup_scripts` and `systemd_units` against its model and
/// kind's built-in image. The client does not know the images on the server, so the server
/// checks again against the image it builds the node from.
///
/// # Errors
///
/// Fails when a node sets a `kind` its model does not take, or sets one of these settings its
/// first-boot config does not take.
pub fn check_file_settings(nodes: &[topology::Node]) -> Result<()> {
    for node in nodes {
        let model_kind = ModelKind::new(node.model, node.kind)
            .with_context(|| format!("Manifest - node: '{}'", node.name))?;
        check_node_file_settings(node, &model_kind.node_config())?;
    }
    Ok(())
}
