use anyhow::{Result, bail};

use crate::{Node, NodeExpanded};
use shared::data::kind_settings::{
    self, ALL, CONTAINER, CONTAINER_GROUP, KindSetting, UNIKERNEL, UNIKERNEL_GROUP, VM,
    VM_AND_CONTAINER, VM_AND_UNIKERNEL, VM_GROUP,
};
use shared::data::model_kinds_list;

/// Check that a node's `kind`, when it sets one, is a kind its model takes. A node that sets
/// none takes its model's default kind; `generic` has none, so its nodes must set one.
///
/// # Errors
///
/// Fails when the node sets a kind its model does not take, or sets none for a model without a
/// default kind, naming the kinds the model takes.
pub fn check_node_model_kind(node: &Node) -> Result<()> {
    if node.kind.is_none() && node.model.default_kind().is_none() {
        bail!(
            "Manifest - node: '{}' sets no `kind`, which '{}' needs; set it to one of: {}",
            node.name,
            node.model,
            model_kinds_list(node.model)
        );
    }
    if let Some(kind) = node.kind
        && !node.model.kinds().contains(&kind)
    {
        bail!(
            r#"Manifest - node: '{}' sets `kind = "{kind}"`, but '{}' comes only as: {}"#,
            node.name,
            node.model,
            model_kinds_list(node.model)
        );
    }
    Ok(())
}

/// Check that a node only sets what its kind takes: the kind groups (`vm`, `container`,
/// `unikernel`) must match the node's kind, and each shared setting must apply to it.
///
/// # Errors
///
/// Fails on the first setting the node's kind does not take, naming the node, the key and the
/// kind.
pub fn check_node_kind_settings(node: &NodeExpanded) -> Result<()> {
    let kind = node.model_kind.kind();
    let cpu = &node.cpu;
    let disk = &node.boot_disk;
    // Each key the node sets, and the kinds of node that take it.
    let settings: [KindSetting; 30] = [
        ("memory", node.memory.is_some(), ALL),
        ("cpu.count", cpu.count.is_some(), ALL),
        ("cpu.mode", cpu.mode.is_some(), VM),
        ("cpu.topology", cpu.topology.is_some(), VM),
        ("cpu.pinning", cpu.pinning.is_some(), VM),
        ("cpu.emulator_pinning", cpu.emulator_pinning.is_some(), VM),
        ("numa.node", node.numa.node.is_some(), VM),
        ("numa.memory_mode", node.numa.memory_mode.is_some(), VM),
        ("boot_disk.size", disk.size.is_some(), VM_AND_UNIKERNEL),
        ("boot_disk.clone_method", disk.clone_method.is_some(), VM),
        ("boot_disk.cache", disk.cache.is_some(), VM),
        ("boot_disk.io", disk.io.is_some(), VM),
        (
            "boot_disk.io_thread_cpus",
            disk.io_thread_cpus.is_some(),
            VM,
        ),
        (
            "interfaces.data_count",
            node.interfaces.data_count.is_some(),
            ALL,
        ),
        ("interfaces.queues", node.interfaces.queues.is_some(), VM),
        ("ztp.method", node.ztp.method.is_some(), VM_AND_CONTAINER),
        ("ztp.config", node.ztp.config.is_some(), VM_AND_CONTAINER),
        (
            "ssh.authorized_keys",
            node.ssh.authorized_keys.is_some(),
            VM_AND_CONTAINER,
        ),
        (
            "ssh.authorized_key_files",
            node.ssh.authorized_key_files.is_some(),
            VM_AND_CONTAINER,
        ),
        ("ready_check.port", node.ready_check.port.is_some(), ALL),
        ("ready_check.skip", node.ready_check.skip.is_some(), ALL),
        (VM_GROUP, node.vm.is_some(), VM),
        (CONTAINER_GROUP, node.container.is_some(), CONTAINER),
        (UNIKERNEL_GROUP, node.unikernel.is_some(), UNIKERNEL),
        (
            "environment_variables",
            node.environment_variables.is_some(),
            VM_AND_CONTAINER,
        ),
        ("text_files", node.text_files.is_some(), VM_AND_CONTAINER),
        (
            "binary_files",
            node.binary_files.is_some(),
            VM_AND_CONTAINER,
        ),
        (
            "systemd_units",
            node.systemd_units.is_some(),
            VM_AND_CONTAINER,
        ),
        (
            "startup_scripts",
            node.startup_scripts.is_some(),
            VM_AND_CONTAINER,
        ),
        (
            "user_scripts",
            node.user_scripts.is_some(),
            VM_AND_CONTAINER,
        ),
    ];
    if let Some((key, _, kinds)) = kind_settings::first_not_taken(kind, &settings) {
        let model = node.model_kind.model();
        // A kind of the same model that takes the setting, which the node could set instead.
        match kinds.iter().find(|other| model.kinds().contains(other)) {
            Some(other) => bail!(
                r#"Manifest - node: '{}' sets `{key}`, which does not apply to {kind} nodes; '{model}' is a {kind} unless the node sets `kind = "{other}"`"#,
                node.name,
            ),
            None => bail!(
                "Manifest - node: '{}' sets `{key}`, which does not apply to {kind} nodes; '{model}' is a {kind}",
                node.name,
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Manifest, NodeContainer};
    use shared::data::{ModelKind, NodeKind, NodeModel};

    /// The one node of a manifest written in TOML, as read.
    fn manifest_node(node: &str) -> Node {
        let manifest: Manifest = toml::from_str(&format!(
            r#"name = "lab"

[[nodes]]
{node}"#
        ))
        .expect("the manifest reads");
        manifest.nodes[0].clone()
    }

    fn node(model: NodeModel) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_string(),
            model_kind: ModelKind::default_for(model),
            ..NodeExpanded::default()
        }
    }

    fn error(node: &NodeExpanded) -> String {
        check_node_kind_settings(node).unwrap_err().to_string()
    }

    #[test]
    fn a_container_group_on_a_vm_is_refused() {
        let mut vm = node(NodeModel::UbuntuLinux);
        vm.container = Some(NodeContainer::default());
        assert_eq!(
            error(&vm),
            "Manifest - node: 'node1' sets `container`, which does not apply to virtual_machine \
             nodes; 'ubuntu_linux' is a virtual_machine"
        );
    }

    #[test]
    fn a_kind_the_model_does_not_take_is_refused_naming_the_kinds_it_takes() {
        let node = manifest_node(
            r#"name = "fw1"
model = "cisco_asa"
kind = "container""#,
        );
        assert_eq!(
            check_node_model_kind(&node).unwrap_err().to_string(),
            r#"Manifest - node: 'fw1' sets `kind = "container"`, but 'cisco_asa' comes only as: virtual_machine"#
        );
        let error = NodeExpanded::new(1, &node).unwrap_err();
        assert!(format!("{error:#}").contains("'fw1'"), "{error:#}");
    }

    #[test]
    fn another_kinds_group_on_a_merged_model_says_which_kind_takes_it() {
        let node = manifest_node(
            r#"name = "r1"
model = "arista_eos"
[nodes.container]
privileged = true"#,
        );
        let expanded = NodeExpanded::new(1, &node).unwrap();
        assert_eq!(
            error(&expanded),
            r#"Manifest - node: 'r1' sets `container`, which does not apply to virtual_machine nodes; 'arista_eos' is a virtual_machine unless the node sets `kind = "container"`"#
        );
    }

    #[test]
    fn generic_without_a_kind_is_refused() {
        let node = manifest_node(
            r#"name = "g1"
model = "generic""#,
        );
        assert_eq!(
            check_node_model_kind(&node).unwrap_err().to_string(),
            "Manifest - node: 'g1' sets no `kind`, which 'generic' needs; set it to one of: \
             virtual_machine, container, unikernel"
        );
        assert!(NodeExpanded::new(1, &node).is_err());
        for kind in NodeKind::to_vec() {
            let node = manifest_node(&format!(
                r#"name = "g1"
model = "generic"
kind = "{kind}""#
            ));
            check_node_model_kind(&node).unwrap();
            assert_eq!(NodeExpanded::new(1, &node).unwrap().model_kind.kind(), kind);
        }
    }

    #[test]
    fn a_removed_model_name_is_refused_when_read_naming_its_model_and_kind() {
        let manifest: Result<Manifest, _> = toml::from_str(
            r#"name = "lab"

[[nodes]]
name = "r1"
model = "arista_ceos""#,
        );
        let error = manifest.unwrap_err().to_string();
        assert!(
            error.contains(
                r#"Unknown node model 'arista_ceos': it was renamed in 0.7.0. Use model = "arista_eos" with kind = "container"."#
            ),
            "{error}"
        );
    }
}
