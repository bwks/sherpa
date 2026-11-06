use anyhow::{Result, bail};

use crate::NodeExpanded;
use shared::data::{NodeConfig, NodeKind};

/// Check a node's `watchdog`: only virtual machines have a hardware watchdog.
///
/// # Errors
///
/// Fails when a node that is not a virtual machine sets `watchdog`.
pub fn check_watchdog(node: &NodeExpanded, image: &NodeConfig) -> Result<()> {
    if node.watchdog.is_some() && image.kind != NodeKind::VirtualMachine {
        bail!(
            "Manifest - node: '{}' sets `watchdog`, which is only for virtual machines; '{}' is a {}",
            node.name,
            image.model,
            image.kind
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{NodeModel, WatchdogAction};

    fn node(model: NodeModel, watchdog: Option<WatchdogAction>) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_string(),
            model,
            watchdog,
            ..NodeExpanded::default()
        }
    }

    #[test]
    fn a_watchdog_on_a_vm_is_accepted() {
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        check_watchdog(
            &node(NodeModel::UbuntuLinux, Some(WatchdogAction::Reset)),
            &image,
        )
        .unwrap();
    }

    #[test]
    fn no_watchdog_is_accepted_on_any_node() {
        for model in [NodeModel::UbuntuLinux, NodeModel::AristaCeos] {
            let image = NodeConfig::get_model(model);
            check_watchdog(&node(model, None), &image).unwrap();
        }
    }

    #[test]
    fn a_watchdog_on_a_container_is_refused_with_the_node_name() {
        let image = NodeConfig::get_model(NodeModel::AristaCeos);
        let error = check_watchdog(
            &node(NodeModel::AristaCeos, Some(WatchdogAction::Reset)),
            &image,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("'node1'"), "{error}");
        assert!(error.contains("only for virtual machines"), "{error}");
    }
}
