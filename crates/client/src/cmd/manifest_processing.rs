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
