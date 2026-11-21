use anyhow::{Context, Result};

use crate::NodeExpanded;
use shared::validators::check_ztp;

/// Check a node's `ztp.method` against the methods its model and kind take. A node that does not set it runs
/// with its image's setting, which is checked where the image is known.
///
/// # Errors
///
/// Fails when the node sets a method its model does not take.
pub fn check_node_ztp(node: &NodeExpanded) -> Result<()> {
    match node.ztp.method {
        Some(ztp) => check_ztp(node.model_kind, ztp)
            .with_context(|| format!("Manifest - node: '{}' sets `ztp.method`", node.name)),
        None => Ok(()),
    }
}
