use anyhow::{Context, Result};

use crate::NodeExpanded;
use shared::validators::check_ztp;

/// Check a node's `ztp` against the methods its model takes. A node that does not set it runs
/// with its image's setting, which is checked where the image is known.
///
/// # Errors
///
/// Fails when the node sets a method its model does not take.
pub fn check_node_ztp(node: &NodeExpanded) -> Result<()> {
    match node.ztp {
        Some(ztp) => check_ztp(node.model, ztp)
            .with_context(|| format!("Manifest - node: '{}' sets `ztp`", node.name)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{NodeModel, Ztp};

    fn node(model: NodeModel, ztp: Option<Ztp>) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_string(),
            model,
            ztp,
            ..NodeExpanded::default()
        }
    }

    #[test]
    fn a_node_without_ztp_is_accepted() {
        check_node_ztp(&node(NodeModel::MongoDb, None)).unwrap();
    }

    #[test]
    fn disabled_and_a_method_the_model_lists_are_accepted() {
        check_node_ztp(&node(NodeModel::FrrLinux, Some(Ztp::Disabled))).unwrap();
        check_node_ztp(&node(NodeModel::JuniperVsrxv3, Some(Ztp::Tftp))).unwrap();
        check_node_ztp(&node(NodeModel::JuniperVsrxv3, Some(Ztp::Cdrom))).unwrap();
    }

    #[test]
    fn a_method_the_model_does_not_list_is_refused_with_the_node_named() {
        let error = check_node_ztp(&node(NodeModel::FrrLinux, Some(Ztp::Cdrom))).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("'node1'"), "{error}");
        assert!(error.contains("'cdrom'"), "{error}");
        assert!(error.contains("allows: disabled, volume"), "{error}");
    }
}
