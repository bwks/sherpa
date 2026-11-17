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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeZtp;
    use shared::data::{ModelKind, NodeModel, Ztp};

    fn node(model: NodeModel, method: Option<Ztp>) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_string(),
            model_kind: ModelKind::default_for(model),
            ztp: NodeZtp {
                method,
                config: None,
            },
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
        check_node_ztp(&node(NodeModel::JuniperSrxv3, Some(Ztp::Cdrom))).unwrap();
    }

    #[test]
    fn srx_with_tftp_is_refused_naming_cdrom() {
        let error = check_node_ztp(&node(NodeModel::JuniperSrxv3, Some(Ztp::Tftp))).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("'node1'"), "{error}");
        assert!(error.contains("'tftp'"), "{error}");
        assert!(error.ends_with("allows: disabled, cdrom"), "{error}");
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
