use anyhow::{Result, bail};

use crate::{Node, NodeExpanded};
use shared::data::kind_settings::{
    self, ALL, CONTAINER, CONTAINER_GROUP, KindSetting, UNIKERNEL, UNIKERNEL_GROUP, VM,
    VM_AND_CONTAINER, VM_AND_UNIKERNEL, VM_GROUP,
};
use shared::data::model_kinds_list;
use shared::data::{NodeConfig, Ztp};
use shared::validators::effective_ztp;

/// The first-boot configs that take `text_files`: cloud-init (cloud-init and cloudbase-init)
/// and Ignition.
const TEXT_FILES_ZTP: &[Ztp] = &[Ztp::CloudInit, Ztp::Ignition];
/// The first-boot configs that take `startup_scripts`: cloud-init and cloudbase-init.
const STARTUP_SCRIPTS_ZTP: &[Ztp] = &[Ztp::CloudInit];
/// The first-boot configs that take `systemd_units`: Ignition.
const SYSTEMD_UNITS_ZTP: &[Ztp] = &[Ztp::Ignition];

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
    let name = &node.name;
    let model = image.model;
    let kind = image.kind();
    let uses = if model.kinds().len() > 1 {
        format!("{model} does not use as a {kind}")
    } else {
        format!("{model} does not use")
    };
    if own_config {
        bail!("Manifest - node: '{name}' sets {key}, which {uses} with its own `ztp.config`");
    }
    let model_takes_it = image
        .model_kind()?
        .ztp_methods()
        .iter()
        .any(|method| takes.contains(method));
    if ztp == Ztp::Disabled && model_takes_it {
        bail!("Manifest - node: '{name}' sets {key}, which {uses} with ZTP disabled");
    }
    bail!("Manifest - node: '{name}' sets {key}, which {uses}");
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
    fn file_setting_error(model_kind: ModelKind, lines: &str, setting: &str) -> Option<String> {
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
            .map(|error| error.to_string())
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
    fn node_settings_001_a_container_group_on_a_virtual_machine_is_refused() {
        let mut vm = node(NodeModel::UbuntuLinux);
        vm.container = Some(NodeContainer::default());
        assert_eq!(
            error(&vm),
            "Manifest - node: 'node1' sets `container`, which does not apply to virtual_machine \
             nodes; 'ubuntu_linux' is a virtual_machine"
        );
    }

    #[test]
    fn node_kind_003_a_kind_the_model_does_not_take_is_refused_naming_the_kinds_it_comes_as() {
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
            r#"Manifest - node: 'r1' sets `container`, which does not apply to virtual_machine nodes; 'arista_eos' is a virtual_machine unless the node sets `kind = "container"`"#
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
    fn node_settings_016_file_settings_on_a_container_are_refused_naming_the_node_the_setting_and_the_model()
     {
        let container = ModelKind::new(NodeModel::Generic, Some(NodeKind::Container)).unwrap();
        for (key, setting) in FILE_SETTINGS {
            assert_eq!(
                file_setting_error(container, "", setting).as_deref(),
                Some(format!("Manifest - node: 'n1' sets {key}, which generic does not use as a container").as_str()),
            );
        }
    }

    #[test]
    fn node_settings_017_file_settings_on_a_network_device_vm_are_refused() {
        let veos = ModelKind::default_for(NodeModel::AristaEos);
        for (key, setting) in FILE_SETTINGS {
            assert_eq!(
                file_setting_error(veos, "", setting).as_deref(),
                Some(format!("Manifest - node: 'n1' sets {key}, which arista_eos does not use as a virtual_machine").as_str()),
            );
        }
        let srx = ModelKind::default_for(NodeModel::JuniperSrxv3);
        let error = file_setting_error(srx, "", FILE_SETTINGS[0].1);
        assert_eq!(
            error.as_deref(),
            Some("Manifest - node: 'n1' sets text_files, which juniper_srxv3 does not use"),
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
                file_setting_error(ModelKind::default_for(model), off, setting).as_deref(),
                Some(
                    format!(
                        "Manifest - node: 'n1' sets {key}, which {model} does not use with ZTP disabled"
                    )
                    .as_str()
                ),
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
                file_setting_error(ModelKind::default_for(model), own, setting).as_deref(),
                Some(
                    format!(
                        "Manifest - node: 'n1' sets {key}, which {model} does not use with its own `ztp.config`"
                    )
                    .as_str()
                ),
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
            )
            .as_deref(),
            Some("Manifest - node: 'n1' sets startup_scripts, which flatcar_linux does not use"),
        );
        for model in [NodeModel::UbuntuLinux, NodeModel::WindowsServer] {
            assert_eq!(
                file_setting_error(ModelKind::default_for(model), "", systemd_units).as_deref(),
                Some(
                    format!("Manifest - node: 'n1' sets systemd_units, which {model} does not use")
                        .as_str()
                ),
            );
        }
    }
}
