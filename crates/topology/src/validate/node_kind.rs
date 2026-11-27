use anyhow::{Result, bail};

use crate::{Node, NodeExpanded, TopologyError};
use shared::data::kind_settings::{
    self, ALL, CONTAINER, CONTAINER_GROUP, KindSetting, UNIKERNEL, UNIKERNEL_GROUP, VM,
    VM_AND_CONTAINER, VM_AND_UNIKERNEL, VM_GROUP,
};
use shared::data::{NodeConfig, NodeModel, Ztp};
use shared::validators::effective_ztp;

/// The first-boot configs that take `text_files`: cloud-init (cloud-init and cloudbase-init)
/// and Ignition.
const TEXT_FILES_ZTP: &[Ztp] = &[Ztp::CloudInit, Ztp::Ignition];
/// The first-boot configs that take `startup_scripts`: cloud-init and cloudbase-init.
const STARTUP_SCRIPTS_ZTP: &[Ztp] = &[Ztp::CloudInit];
/// The first-boot configs that take `systemd_units`: Ignition.
const SYSTEMD_UNITS_ZTP: &[Ztp] = &[Ztp::Ignition];

/// Check that a node's model is a node model, and that its `kind`, when it sets one, is a kind
/// its model takes. A node that sets none takes its model's default kind; `generic` has none, so
/// its nodes must set one.
///
/// # Errors
///
/// Fails when the node's model is `flatcar_sandbox`, the sandbox base image, when it sets a kind
/// its model does not take, or sets none for a model without a default kind, naming the kinds
/// the model takes.
pub fn check_node_model_kind(node: &Node) -> Result<()> {
    if node.model == NodeModel::FlatcarSandbox {
        bail!(TopologyError::SandboxBaseModel {
            node: node.name.clone(),
        });
    }
    if node.kind.is_none() && node.model.default_kind().is_none() {
        bail!(TopologyError::KindMissing {
            node: node.name.clone(),
            model: node.model,
            kinds: node.model.kinds().to_vec(),
        });
    }
    if let Some(kind) = node.kind
        && !node.model.kinds().contains(&kind)
    {
        bail!(TopologyError::KindNotTaken {
            node: node.name.clone(),
            kind,
            model: node.model,
            kinds: node.model.kinds().to_vec(),
        });
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
    let settings: [KindSetting; 23] = [
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
    ];
    if let Some((key, _, kinds)) = kind_settings::first_not_taken(kind, &settings) {
        let model = node.model_kind.model();
        // A kind of the same model that takes the setting, which the node could set instead.
        let other = kinds
            .iter()
            .find(|other| model.kinds().contains(other))
            .copied();
        bail!(TopologyError::SettingNotForKind {
            node: node.name.clone(),
            key: (*key).to_owned(),
            kind,
            model,
            other,
        });
    }
    Ok(())
}

/// Check that a node sets `text_files`, `startup_scripts` and `systemd_units` only when the
/// first-boot config Sherpa generates for it carries them. That config follows from the ZTP
/// setting the node runs with, the manifest's or else its `image`'s, which its model data
/// bounds: cloud-init, on Linux, BSD and Windows (cloudbase-init), takes `text_files` and
/// `startup_scripts`, and Ignition, on Flatcar, takes `text_files` and `systemd_units`. Every
/// other setting takes none: ZTP off, the methods of network devices and every container's. A
/// node's own `ztp.config` replaces the generated config, so it takes none either.
///
/// # Errors
///
/// Fails on the first of these settings the node sets that its config does not take, naming
/// the node, the setting and the model, and why when the model would take it with ZTP on or
/// without its own config; or when `image` is of a kind its model does not take.
pub fn check_node_file_settings(node: &Node, image: &NodeConfig) -> Result<()> {
    let ztp = effective_ztp(node.ztp.method, image);
    let own_config = node.ztp.config.is_some();
    let settings: [(&str, bool, &[Ztp]); 3] = [
        ("text_files", node.text_files.is_some(), TEXT_FILES_ZTP),
        (
            "startup_scripts",
            node.startup_scripts.is_some(),
            STARTUP_SCRIPTS_ZTP,
        ),
        (
            "systemd_units",
            node.systemd_units.is_some(),
            SYSTEMD_UNITS_ZTP,
        ),
    ];
    let Some((key, _, takes)) = settings
        .iter()
        .find(|(_, set, takes)| *set && (own_config || !takes.contains(&ztp)))
    else {
        return Ok(());
    };
    let node_name = node.name.clone();
    let key = *key;
    let model = image.model;
    let kind = image.kind();
    if own_config {
        bail!(TopologyError::FileSettingWithOwnConfig {
            node: node_name,
            key,
            model,
            kind,
        });
    }
    let model_takes_it = image
        .model_kind()?
        .ztp_methods()
        .iter()
        .any(|method| takes.contains(method));
    if ztp == Ztp::Disabled && model_takes_it {
        bail!(TopologyError::FileSettingWithZtpDisabled {
            node: node_name,
            key,
            model,
            kind,
        });
    }
    bail!(TopologyError::FileSettingNotUsed {
        node: node_name,
        key,
        model,
        kind,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Manifest, NodeContainer};
    use shared::data::{ModelKind, NodeKind, NodeModel};

    /// Each of the settings, as a node's TOML lines.
    const FILE_SETTINGS: [(&str, &str); 3] = [
        (
            "text_files",
            r#"[[nodes.text_files]]
src = "motd"
dst = "/etc/motd"
user = "root"
group = "root"
permissions = 644"#,
        ),
        ("startup_scripts", r#"startup_scripts = ["boot.sh"]"#),
        (
            "systemd_units",
            r#"[[nodes.systemd_units]]
name = "app.service"
source = "app.service"
enabled = true"#,
        ),
    ];

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

    /// The refusal of `check_node_file_settings` for a node of `model` and `kind` with `lines`
    /// and the `setting` in it, or `None` when it is taken, checked against the model and
    /// kind's built-in image.
    fn file_setting_error(
        model_kind: ModelKind,
        lines: &str,
        setting: &str,
    ) -> Option<TopologyError> {
        let node = manifest_node(&format!(
            r#"name = "n1"
model = "{}"
kind = "{}"
{lines}
{setting}"#,
            model_kind.model(),
            model_kind.kind(),
        ));
        check_node_file_settings(&node, &model_kind.node_config())
            .err()
            .map(|error| error.downcast::<TopologyError>().expect("a topology error"))
    }

    /// The refusal of a file setting `key` that `model` as `kind` does not use.
    fn not_used(key: &'static str, model: NodeModel, kind: NodeKind) -> TopologyError {
        TopologyError::FileSettingNotUsed {
            node: "n1".to_owned(),
            key,
            model,
            kind,
        }
    }

    fn node(model: NodeModel) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_string(),
            model_kind: ModelKind::default_for(model),
            ..NodeExpanded::default()
        }
    }

    fn error(node: &NodeExpanded) -> TopologyError {
        check_node_kind_settings(node)
            .unwrap_err()
            .downcast::<TopologyError>()
            .expect("a topology error")
    }

    #[test]
    fn node_settings_001_a_container_group_on_a_virtual_machine_is_refused() {
        let mut vm = node(NodeModel::UbuntuLinux);
        vm.container = Some(NodeContainer::default());
        assert_eq!(
            error(&vm),
            TopologyError::SettingNotForKind {
                node: "node1".to_owned(),
                key: "container".to_owned(),
                kind: NodeKind::VirtualMachine,
                model: NodeModel::UbuntuLinux,
                other: None,
            }
        );
    }

    #[test]
    fn node_kind_010_a_flatcar_sandbox_node_is_refused_pointing_to_flatcar_linux() {
        let node = manifest_node(
            r#"name = "fc1"
model = "flatcar_sandbox""#,
        );
        let error = check_node_model_kind(&node).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::SandboxBaseModel {
                node: "fc1".to_owned(),
            })
        );
    }

    #[test]
    fn node_kind_003_a_kind_the_model_does_not_take_is_refused_naming_the_kinds_it_comes_as() {
        let node = manifest_node(
            r#"name = "fw1"
model = "cisco_asa"
kind = "container""#,
        );
        let error = check_node_model_kind(&node).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::KindNotTaken {
                node: "fw1".to_owned(),
                kind: NodeKind::Container,
                model: NodeModel::CiscoAsa,
                kinds: vec![NodeKind::VirtualMachine],
            })
        );
        let error = NodeExpanded::new(1, &node).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::NodeModelKind {
                node: "fw1".to_owned(),
            })
        );
    }

    #[test]
    fn node_settings_002_on_a_model_of_several_kinds_the_refusal_names_the_kind_that_takes_it() {
        let node = manifest_node(
            r#"name = "r1"
model = "arista_eos"
[nodes.container]
privileged = true"#,
        );
        let expanded = NodeExpanded::new(1, &node).unwrap();
        assert_eq!(
            error(&expanded),
            TopologyError::SettingNotForKind {
                node: "r1".to_owned(),
                key: "container".to_owned(),
                kind: NodeKind::VirtualMachine,
                model: NodeModel::AristaEos,
                other: Some(NodeKind::Container),
            }
        );
    }

    #[test]
    fn node_kind_001_a_node_without_kind_is_its_models_default_kind() {
        for model in ["arista_eos", "juniper_evolved", "sonic_linux"] {
            let node = manifest_node(&format!(
                r#"name = "r1"
model = "{model}""#
            ));
            check_node_model_kind(&node).unwrap();
            assert_eq!(
                NodeExpanded::new(1, &node).unwrap().model_kind.kind(),
                NodeKind::VirtualMachine,
                "{model}"
            );
        }
    }

    #[test]
    fn node_kind_002_a_generic_node_without_a_kind_is_refused() {
        let node = manifest_node(
            r#"name = "g1"
model = "generic""#,
        );
        let error = check_node_model_kind(&node).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::KindMissing {
                node: "g1".to_owned(),
                model: NodeModel::Generic,
                kinds: vec![
                    NodeKind::VirtualMachine,
                    NodeKind::Container,
                    NodeKind::Unikernel,
                ],
            })
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
    fn node_settings_016_file_settings_on_a_container_are_refused_naming_the_node_the_setting_and_the_model()
     {
        let container = ModelKind::new(NodeModel::Generic, Some(NodeKind::Container)).unwrap();
        for (key, setting) in FILE_SETTINGS {
            assert_eq!(
                file_setting_error(container, "", setting),
                Some(not_used(key, NodeModel::Generic, NodeKind::Container)),
            );
        }
    }

    #[test]
    fn node_settings_017_file_settings_on_a_network_device_vm_are_refused() {
        let veos = ModelKind::default_for(NodeModel::AristaEos);
        for (key, setting) in FILE_SETTINGS {
            assert_eq!(
                file_setting_error(veos, "", setting),
                Some(not_used(
                    key,
                    NodeModel::AristaEos,
                    NodeKind::VirtualMachine
                )),
            );
        }
        let srx = ModelKind::default_for(NodeModel::JuniperSrxv3);
        assert_eq!(
            file_setting_error(srx, "", FILE_SETTINGS[0].1),
            Some(not_used(
                "text_files",
                NodeModel::JuniperSrxv3,
                NodeKind::VirtualMachine
            )),
        );
    }

    #[test]
    fn node_settings_018_file_settings_on_a_node_with_ztp_off_are_refused() {
        let off = r#"ztp = { method = "disabled" }"#;
        let [text_files, startup_scripts, systemd_units] = FILE_SETTINGS;
        let used_with_ztp_on = [
            (NodeModel::UbuntuLinux, text_files),
            (NodeModel::UbuntuLinux, startup_scripts),
            (NodeModel::FlatcarLinux, systemd_units),
        ];
        for (model, (key, setting)) in used_with_ztp_on {
            assert_eq!(
                file_setting_error(ModelKind::default_for(model), off, setting),
                Some(TopologyError::FileSettingWithZtpDisabled {
                    node: "n1".to_owned(),
                    key,
                    model,
                    kind: NodeKind::VirtualMachine,
                }),
            );
        }
    }

    #[test]
    fn node_settings_019_file_settings_on_a_node_with_its_own_ztp_config_are_refused() {
        let own = r#"ztp = { config = "user-data.yaml" }"#;
        let [text_files, startup_scripts, systemd_units] = FILE_SETTINGS;
        let used_without_it = [
            (NodeModel::UbuntuLinux, text_files),
            (NodeModel::UbuntuLinux, startup_scripts),
            (NodeModel::FlatcarLinux, systemd_units),
        ];
        for (model, (key, setting)) in used_without_it {
            assert_eq!(
                file_setting_error(ModelKind::default_for(model), own, setting),
                Some(TopologyError::FileSettingWithOwnConfig {
                    node: "n1".to_owned(),
                    key,
                    model,
                    kind: NodeKind::VirtualMachine,
                }),
            );
        }
    }

    #[test]
    fn node_settings_020_file_settings_are_taken_where_the_first_boot_config_carries_them() {
        let [text_files, startup_scripts, systemd_units] = FILE_SETTINGS.map(|(_, lines)| lines);
        let taken = [
            (NodeModel::UbuntuLinux, text_files),
            (NodeModel::UbuntuLinux, startup_scripts),
            (NodeModel::AlpineLinux, startup_scripts),
            (NodeModel::FreeBsd, text_files),
            (NodeModel::WindowsServer, text_files),
            (NodeModel::WindowsServer, startup_scripts),
            (NodeModel::FlatcarLinux, text_files),
            (NodeModel::FlatcarLinux, systemd_units),
        ];
        for (model, setting) in taken {
            assert_eq!(
                file_setting_error(ModelKind::default_for(model), "", setting),
                None,
                "{model}: {setting}"
            );
        }
        let generic_vm =
            ModelKind::new(NodeModel::Generic, Some(NodeKind::VirtualMachine)).unwrap();
        assert_eq!(file_setting_error(generic_vm, "", text_files), None);
    }

    #[test]
    fn node_settings_021_startup_scripts_on_flatcar_and_systemd_units_on_cloud_init_are_refused() {
        let [_, startup_scripts, systemd_units] = FILE_SETTINGS.map(|(_, lines)| lines);
        assert_eq!(
            file_setting_error(
                ModelKind::default_for(NodeModel::FlatcarLinux),
                "",
                startup_scripts
            ),
            Some(not_used(
                "startup_scripts",
                NodeModel::FlatcarLinux,
                NodeKind::VirtualMachine
            )),
        );
        for model in [NodeModel::UbuntuLinux, NodeModel::WindowsServer] {
            assert_eq!(
                file_setting_error(ModelKind::default_for(model), "", systemd_units),
                Some(not_used("systemd_units", model, NodeKind::VirtualMachine)),
            );
        }
    }
}
