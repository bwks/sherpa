use anyhow::{Result, bail};

use crate::NodeExpanded;
use shared::data::NodeKind;

const ALL: &[NodeKind] = &[
    NodeKind::VirtualMachine,
    NodeKind::Container,
    NodeKind::Unikernel,
];
const VM: &[NodeKind] = &[NodeKind::VirtualMachine];
const CONTAINER: &[NodeKind] = &[NodeKind::Container];
const UNIKERNEL: &[NodeKind] = &[NodeKind::Unikernel];
const VM_AND_CONTAINER: &[NodeKind] = &[NodeKind::VirtualMachine, NodeKind::Container];
const VM_AND_UNIKERNEL: &[NodeKind] = &[NodeKind::VirtualMachine, NodeKind::Unikernel];

/// Check that a node only sets what its kind takes: the kind groups (`vm`, `container`,
/// `unikernel`) must match its model, and each shared setting must apply to its kind.
///
/// # Errors
///
/// Fails on the first setting the node's kind does not take, naming the node, the key and the
/// kind.
pub fn check_node_kind_settings(node: &NodeExpanded) -> Result<()> {
    let kind = node.model.kind();
    let cpu = &node.cpu;
    let disk = &node.boot_disk;
    // Each key the node sets, and the kinds of node that take it.
    let settings: [(&str, bool, &[NodeKind]); 30] = [
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
        (
            "ready_check.port",
            node.ready_check.port.is_some(),
            VM_AND_UNIKERNEL,
        ),
        ("ready_check.skip", node.ready_check.skip.is_some(), ALL),
        ("vm", node.vm.is_some(), VM),
        ("container", node.container.is_some(), CONTAINER),
        ("unikernel", node.unikernel.is_some(), UNIKERNEL),
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
    if let Some((key, _, _)) = settings
        .iter()
        .find(|(_, set, kinds)| *set && !kinds.contains(&kind))
    {
        bail!(
            "Manifest - node: '{}' sets `{key}`, which does not apply to {kind} nodes; '{}' is a {kind}",
            node.name,
            node.model,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeContainer, NodeUnikernel, NodeVm, StartupScript};
    use shared::data::{
        DiskCache, NodeBootDisk, NodeCpu, NodeInterfaces, NodeModel, NodeNuma, WatchdogAction, Ztp,
    };

    fn node(model: NodeModel) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_string(),
            model,
            ..NodeExpanded::default()
        }
    }

    fn error(node: &NodeExpanded) -> String {
        check_node_kind_settings(node).unwrap_err().to_string()
    }

    #[test]
    fn a_vm_takes_every_vm_setting() {
        let mut vm = node(NodeModel::UbuntuLinux);
        vm.memory = Some(2048);
        vm.cpu = NodeCpu {
            count: Some(2),
            pinning: Some(vec![1, 2]),
            ..NodeCpu::default()
        };
        vm.numa = NodeNuma {
            node: Some(0),
            ..NodeNuma::default()
        };
        vm.boot_disk = NodeBootDisk {
            size: Some(20),
            ..NodeBootDisk::default()
        };
        vm.interfaces = NodeInterfaces {
            data_count: Some(2),
            queues: Some(2),
        };
        vm.ready_check.port = Some(22);
        vm.vm = Some(NodeVm {
            watchdog: Some(WatchdogAction::Reset),
        });
        check_node_kind_settings(&vm).unwrap();
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
    fn a_vm_group_on_a_container_is_refused() {
        let mut container = node(NodeModel::GenericContainer);
        container.vm = Some(NodeVm {
            watchdog: Some(WatchdogAction::Reset),
        });
        let message = error(&container);
        assert!(message.contains("'node1' sets `vm`"), "{message}");
        assert!(message.contains("container nodes"), "{message}");
    }

    #[test]
    fn a_unikernel_group_on_a_vm_is_refused() {
        let mut vm = node(NodeModel::UbuntuLinux);
        vm.unikernel = Some(NodeUnikernel::default());
        assert!(error(&vm).contains("sets `unikernel`"));
    }

    #[test]
    fn cpu_count_and_memory_on_a_container_are_accepted() {
        let mut container = node(NodeModel::GenericContainer);
        container.cpu.count = Some(2);
        container.memory = Some(512);
        check_node_kind_settings(&container).unwrap();
    }

    #[test]
    fn a_container_takes_its_settings() {
        let mut container = node(NodeModel::GenericContainer);
        container.interfaces.data_count = Some(2);
        container.ready_check.skip = Some(true);
        container.container = Some(NodeContainer {
            privileged: Some(true),
            ..NodeContainer::default()
        });
        container.startup_scripts = Some(vec![StartupScript::default()]);
        check_node_kind_settings(&container).unwrap();
    }

    #[test]
    fn a_ready_port_on_a_container_is_refused() {
        let mut container = node(NodeModel::GenericContainer);
        container.ready_check.port = Some(80);
        assert!(error(&container).contains("sets `ready_check.port`"));
    }

    #[test]
    fn vm_tuning_on_a_unikernel_is_refused() {
        let mut unikernel = node(NodeModel::NanosUnikernel);
        unikernel.boot_disk.cache = Some(DiskCache::None);
        assert_eq!(
            error(&unikernel),
            "Manifest - node: 'node1' sets `boot_disk.cache`, which does not apply to unikernel \
             nodes; 'nanos_unikernel' is a unikernel"
        );
        let mut unikernel = node(NodeModel::NanosUnikernel);
        unikernel.numa.node = Some(0);
        assert!(error(&unikernel).contains("sets `numa.node`"));
    }

    #[test]
    fn a_unikernel_takes_its_settings() {
        let mut unikernel = node(NodeModel::NanosUnikernel);
        unikernel.memory = Some(512);
        unikernel.cpu.count = Some(1);
        unikernel.boot_disk.size = Some(2);
        unikernel.ready_check.port = Some(8080);
        unikernel.ready_check.skip = Some(false);
        unikernel.unikernel = Some(NodeUnikernel {
            cmdline: Some("console=ttyS0".to_string()),
        });
        check_node_kind_settings(&unikernel).unwrap();
    }

    #[test]
    fn ztp_and_files_on_a_unikernel_are_refused() {
        let mut unikernel = node(NodeModel::UnikraftUnikernel);
        unikernel.ztp.method = Some(Ztp::Disabled);
        assert!(error(&unikernel).contains("sets `ztp.method`"));
        let mut unikernel = node(NodeModel::UnikraftUnikernel);
        unikernel.environment_variables = Some(vec!["A=1".to_string()]);
        assert!(error(&unikernel).contains("sets `environment_variables`"));
    }
}
