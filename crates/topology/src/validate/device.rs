use std::collections::HashMap;

use anyhow::{Result, bail};

use crate::{Node, TopologyError};
use shared::data::ModelKind;
use shared::konst::{MANAGEMENT_NODE_HOST_OFFSETS, SANDBOX_HOST_SUFFIX};

/// Whether `node` runs in a sandbox; a node whose model does not take its kind does not, and
/// another check refuses it.
fn runs_in_sandbox(node: &Node) -> bool {
    ModelKind::new(node.model, node.kind).is_ok_and(ModelKind::runs_in_sandbox)
}

/// Check no two nodes share a name, in any case, as DNS takes names that differ only in case as
/// one, nor the name of another node's sandbox, its name with [`SANDBOX_HOST_SUFFIX`].
///
/// # Errors
///
/// Fails when a node name is used more than once, two names differ only in case, or a node has
/// the name of another node's sandbox.
pub fn check_duplicate_device(devices: &[Node]) -> Result<()> {
    for sandbox in devices.iter().filter(|device| runs_in_sandbox(device)) {
        let taken = format!("{}{SANDBOX_HOST_SUFFIX}", sandbox.name).to_ascii_lowercase();
        if let Some(node) = devices
            .iter()
            .find(|device| device.name.to_ascii_lowercase() == taken)
        {
            bail!(TopologyError::SandboxNameTaken {
                node: node.name.clone(),
                sandbox: sandbox.name.clone(),
            });
        }
    }
    let mut names: HashMap<String, &str> = HashMap::new();
    for device in devices {
        if let Some(other) = names.insert(device.name.to_ascii_lowercase(), &device.name) {
            if other == device.name {
                bail!(TopologyError::DuplicateNode {
                    node: device.name.clone(),
                });
            }
            bail!(TopologyError::NodeNamesDifferInCase {
                node: device.name.clone(),
                other: other.to_owned(),
            });
        }
    }
    Ok(())
}

/// Check the lab management node block has an address for each node and for each node's
/// sandbox, which has an address of its own.
///
/// # Errors
///
/// Fails when there are more nodes, or more nodes and sandboxes, than addresses.
pub fn check_device_count(devices: &[Node]) -> Result<()> {
    let max = MANAGEMENT_NODE_HOST_OFFSETS.clone().count();
    if devices.len() > max {
        bail!(TopologyError::TooManyNodes {
            count: devices.len(),
            max,
        });
    }
    let sandboxes = devices
        .iter()
        .filter(|device| runs_in_sandbox(device))
        .count();
    if devices.len() + sandboxes > max {
        bail!(TopologyError::TooManyAddresses {
            nodes: devices.len(),
            sandboxes,
            max,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use shared::data::NodeModel;

    use super::*;

    #[test]
    fn node_naming_003_two_nodes_with_one_name_are_refused_naming_it() {
        let devices = vec![
            Node {
                name: "router1".to_string(),
                ..Default::default()
            },
            Node {
                name: "router1".to_string(),
                ..Default::default()
            },
        ];
        let error = check_duplicate_device(&devices).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::DuplicateNode {
                node: "router1".to_owned(),
            })
        );
    }

    #[test]
    fn node_naming_007_two_names_that_differ_only_in_case_are_refused_naming_both() {
        let devices = vec![
            Node {
                name: "Router1".to_string(),
                ..Default::default()
            },
            Node {
                name: "router1".to_string(),
                ..Default::default()
            },
        ];
        let error = check_duplicate_device(&devices).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::NodeNamesDifferInCase {
                node: "router1".to_owned(),
                other: "Router1".to_owned(),
            })
        );
    }

    #[test]
    fn node_naming_004_a_manifest_with_235_nodes_is_refused() {
        let devices = vec![Node::default(); 235];
        let error = check_device_count(&devices).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::TooManyNodes {
                count: 235,
                max: 234,
            })
        );
    }

    /// A node of model `model` named `name`, of the model's default kind.
    fn node(name: &str, model: NodeModel) -> Node {
        Node {
            name: name.to_owned(),
            model,
            ..Default::default()
        }
    }

    #[test]
    fn node_naming_009_nodes_and_their_sandboxes_past_the_lab_node_block_are_refused() {
        let mut devices = vec![node("srl", NodeModel::NokiaSrlinux); 117];
        devices
            .iter_mut()
            .enumerate()
            .for_each(|(i, device)| device.name = format!("srl{i}"));
        check_device_count(&devices).unwrap();
        devices.push(node("alpine", NodeModel::AlpineLinux));
        let error = check_device_count(&devices).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::TooManyAddresses {
                nodes: 118,
                sandboxes: 117,
                max: 234,
            })
        );
    }

    #[test]
    fn node_naming_010_a_node_named_as_another_nodes_sandbox_is_refused() {
        let devices = vec![
            node("srl1", NodeModel::NokiaSrlinux),
            node("SRL1-sbx", NodeModel::AlpineLinux),
        ];
        let error = check_duplicate_device(&devices).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::SandboxNameTaken {
                node: "SRL1-sbx".to_owned(),
                sandbox: "srl1".to_owned(),
            })
        );
        check_duplicate_device(&[
            node("alp1", NodeModel::AlpineLinux),
            node("alp1-sbx", NodeModel::AlpineLinux),
        ])
        .unwrap();
    }
}
