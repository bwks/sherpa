use anyhow::{Result, bail};

use crate::NodeExpanded;
use shared::data::{DiskBuses, DiskCache, DiskIo, InterfaceType, NodeConfig, NodeCpuPins};

/// Check a VM's CPU, NUMA, boot disk and interface tuning against its own settings and its
/// image. Other kinds do not take these settings, which `check_node_kind_settings` refuses.
///
/// # Errors
///
/// Fails when a setting does not fit the node's CPU count, disks or interfaces.
pub fn check_vm_tuning(node: &NodeExpanded, image: &NodeConfig) -> Result<()> {
    let Some(vm) = image.vm() else {
        return Ok(());
    };
    let name = &node.name;
    let cpu = &node.cpu;
    let disk = &node.boot_disk;

    let needs_cpu_count =
        cpu.topology.is_some() || cpu.pinning.is_some() || node.interfaces.queues.is_some();
    let cpu_count = match cpu.count {
        Some(count) => u32::from(count),
        None if needs_cpu_count => bail!(
            "Manifest - node: '{name}' sets `cpu.topology`, `cpu.pinning` or \
             `interfaces.queues`; set `cpu.count` too"
        ),
        None => u32::from(image.cpu.count),
    };

    if let Some(topology) = &cpu.topology {
        let vcpus =
            u32::from(topology.sockets) * u32::from(topology.cores) * u32::from(topology.threads);
        if vcpus != cpu_count {
            bail!(
                "Manifest - node: '{name}' `cpu.topology` gives {vcpus} vCPUs, but `cpu.count` \
                 is {cpu_count}"
            );
        }
    }

    let vcpu_cpus = cpu.pinning.as_deref().unwrap_or_default();
    if let Some(pinning) = &cpu.pinning {
        if u32::try_from(pinning.len()).ok() != Some(cpu_count) {
            bail!(
                "Manifest - node: '{name}' `cpu.pinning` lists {} host CPUs, one is needed for \
                 each of its {cpu_count} vCPUs",
                pinning.len()
            );
        }
        if let Some(repeat) = first_repeat(pinning) {
            bail!("Manifest - node: '{name}' `cpu.pinning` lists host CPU {repeat} more than once");
        }
    }

    for (setting, cpus) in [
        ("cpu.emulator_pinning", &cpu.emulator_pinning),
        ("boot_disk.io_thread_cpus", &disk.io_thread_cpus),
    ] {
        let Some(cpus) = cpus else {
            continue;
        };
        if cpus.is_empty() {
            bail!("Manifest - node: '{name}' `{setting}` is empty; list host CPUs or remove it");
        }
        if let Some(shared) = cpus.iter().find(|host_cpu| vcpu_cpus.contains(host_cpu)) {
            bail!(
                "Manifest - node: '{name}' `{setting}` uses host CPU {shared}, which is pinned to \
                 one of its vCPUs"
            );
        }
    }

    if node.numa.memory_mode.is_some() && node.numa.node.is_none() {
        bail!(
            "Manifest - node: '{name}' sets `numa.memory_mode` without `numa.node`; set the \
             host NUMA node its memory is placed on, or remove `numa.memory_mode`"
        );
    }

    if disk.io == Some(DiskIo::Native) && disk.cache != Some(DiskCache::None) {
        bail!(
            r#"Manifest - node: '{name}' sets `boot_disk.io = "native"`, which needs `boot_disk.cache = "none"`"#
        );
    }

    if disk.io_thread_cpus.is_some() && vm.boot_disk.bus != DiskBuses::Virtio {
        bail!(
            "Manifest - node: '{name}' sets `boot_disk.io_thread_cpus`, which needs virtio \
             disks; '{}' uses {}",
            image.model,
            vm.boot_disk.bus
        );
    }

    if let Some(queues) = node.interfaces.queues {
        if queues == 0 || u32::from(queues) > cpu_count {
            bail!(
                "Manifest - node: '{name}' `interfaces.queues` must be between 1 and \
                 {cpu_count}, its `cpu.count`"
            );
        }
        if vm.interfaces.kind != InterfaceType::Virtio {
            bail!(
                "Manifest - node: '{name}' sets `interfaces.queues`, which needs virtio \
                 interfaces; '{}' uses {:?}",
                image.model,
                vm.interfaces.kind
            );
        }
    }

    Ok(())
}

/// Check that no host CPU pinned to a vCPU is shared: not with another node's vCPUs, and not
/// with any node's QEMU or I/O threads. QEMU and I/O threads may share CPUs with each other.
///
/// # Errors
///
/// Fails when a host CPU pinned to a vCPU is used twice.
pub fn check_cpu_pin_overlap(nodes: &[NodeCpuPins]) -> Result<()> {
    for (index, node) in nodes.iter().enumerate() {
        for other in &nodes[index + 1..] {
            if let Some(cpu) = node
                .vcpu_cpus
                .iter()
                .find(|cpu| other.vcpu_cpus.contains(cpu))
            {
                bail!(
                    "Host CPU {cpu} is pinned to vCPUs of both node '{}' and node '{}'",
                    node.node_name,
                    other.node_name
                );
            }
        }
        for other in nodes {
            if let Some(cpu) = node
                .qemu_thread_cpus
                .iter()
                .find(|cpu| other.vcpu_cpus.contains(cpu))
            {
                bail!(
                    "Host CPU {cpu} runs QEMU threads of node '{}' but is pinned to vCPUs of \
                     node '{}'",
                    node.node_name,
                    other.node_name
                );
            }
        }
    }
    Ok(())
}

/// The first host CPU listed more than once.
fn first_repeat(cpus: &[u16]) -> Option<u16> {
    cpus.iter()
        .enumerate()
        .find(|(index, cpu)| cpus[..*index].contains(cpu))
        .map(|(_, cpu)| *cpu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{CpuTopology, KindSettings, ModelKind, NodeCpu, NodeModel, NumaMemoryMode};

    fn node(cpu_count: Option<u8>) -> NodeExpanded {
        NodeExpanded {
            name: "dev".to_string(),
            model_kind: ModelKind::default_for(NodeModel::UbuntuLinux),
            cpu: NodeCpu {
                count: cpu_count,
                ..NodeCpu::default()
            },
            ..NodeExpanded::default()
        }
    }

    fn ubuntu() -> NodeConfig {
        ModelKind::default_for(NodeModel::UbuntuLinux).node_config()
    }

    fn error(node: &NodeExpanded, image: &NodeConfig) -> String {
        format!(
            "{:#}",
            check_vm_tuning(node, image).expect_err("tuning is rejected")
        )
    }

    fn pins(name: &str, vcpus: &[u16], qemu: &[u16]) -> NodeCpuPins {
        NodeCpuPins {
            node_name: name.to_string(),
            vcpu_cpus: vcpus.to_vec(),
            qemu_thread_cpus: qemu.to_vec(),
        }
    }

    #[test]
    fn vm_tuning_001_a_virtual_machine_with_every_tuning_setting_is_taken() {
        let mut vm = node(Some(4));
        vm.cpu.topology = Some(CpuTopology {
            sockets: 1,
            cores: 2,
            threads: 2,
        });
        vm.cpu.pinning = Some(vec![9, 25, 10, 26]);
        vm.cpu.emulator_pinning = Some(vec![8, 24]);
        vm.numa.node = Some(1);
        vm.numa.memory_mode = Some(NumaMemoryMode::Preferred);
        vm.boot_disk.cache = Some(DiskCache::None);
        vm.boot_disk.io = Some(DiskIo::Native);
        vm.boot_disk.io_thread_cpus = Some(vec![8, 24]);
        vm.interfaces.queues = Some(4);
        check_vm_tuning(&vm, &ubuntu()).unwrap();
    }

    #[test]
    fn vm_tuning_002_pinning_topology_or_queues_without_cpu_count_are_refused() {
        let topology = CpuTopology {
            sockets: 1,
            cores: 1,
            threads: 1,
        };
        let mut pinning = node(None);
        pinning.cpu.pinning = Some(vec![1]);
        let mut with_topology = node(None);
        with_topology.cpu.topology = Some(topology);
        let mut queues = node(None);
        queues.interfaces.queues = Some(1);
        for vm in [pinning, with_topology, queues] {
            let message = error(&vm, &ubuntu());
            assert!(message.contains("set `cpu.count`"), "{message}");
        }
    }

    #[test]
    fn vm_tuning_003_a_topology_that_does_not_multiply_to_cpu_count_is_refused() {
        let mut vm = node(Some(4));
        vm.cpu.topology = Some(CpuTopology {
            sockets: 1,
            cores: 3,
            threads: 2,
        });
        let message = error(&vm, &ubuntu());
        assert!(
            message.contains("gives 6 vCPUs, but `cpu.count` is 4"),
            "{message}"
        );
    }

    #[test]
    fn vm_tuning_004_a_pinning_without_one_host_cpu_per_vcpu_is_refused() {
        let mut short = node(Some(4));
        short.cpu.pinning = Some(vec![1, 2]);
        let message = error(&short, &ubuntu());
        assert!(message.contains("4 vCPUs"), "{message}");
    }

    #[test]
    fn vm_tuning_005_a_pinning_that_lists_a_host_cpu_twice_is_refused_naming_it() {
        let mut repeated = node(Some(4));
        repeated.cpu.pinning = Some(vec![1, 2, 2, 3]);
        let message = error(&repeated, &ubuntu());
        assert!(message.contains("host CPU 2 more than once"), "{message}");
    }

    #[test]
    fn vm_tuning_006_qemu_or_io_threads_on_the_nodes_own_vcpu_cpus_are_refused() {
        let mut emulator = node(Some(2));
        emulator.cpu.pinning = Some(vec![1, 2]);
        emulator.cpu.emulator_pinning = Some(vec![1]);
        let message = error(&emulator, &ubuntu());
        assert!(
            message.contains("`cpu.emulator_pinning` uses host CPU 1"),
            "{message}"
        );

        let mut io_thread = node(Some(2));
        io_thread.cpu.pinning = Some(vec![1, 2]);
        io_thread.boot_disk.io_thread_cpus = Some(vec![2]);
        let message = error(&io_thread, &ubuntu());
        assert!(
            message.contains("`boot_disk.io_thread_cpus` uses host CPU 2"),
            "{message}"
        );
    }

    #[test]
    fn vm_tuning_007_an_empty_emulator_pinning_or_io_thread_cpus_is_refused() {
        let mut emulator = node(None);
        emulator.cpu.emulator_pinning = Some(vec![]);
        let message = error(&emulator, &ubuntu());
        assert!(
            message.contains("`cpu.emulator_pinning` is empty"),
            "{message}"
        );

        let mut io_thread = node(None);
        io_thread.boot_disk.io_thread_cpus = Some(vec![]);
        let message = error(&io_thread, &ubuntu());
        assert!(
            message.contains("`boot_disk.io_thread_cpus` is empty"),
            "{message}"
        );
    }

    #[test]
    fn vm_tuning_008_numa_memory_mode_without_numa_node_is_refused() {
        let mut vm = node(None);
        vm.numa.memory_mode = Some(NumaMemoryMode::Preferred);
        let message = error(&vm, &ubuntu());
        assert!(
            message.contains("sets `numa.memory_mode` without `numa.node`"),
            "{message}"
        );
    }

    #[test]
    fn vm_tuning_009_native_io_without_no_cache_is_refused() {
        let mut vm = node(None);
        vm.boot_disk.io = Some(DiskIo::Native);
        vm.boot_disk.cache = Some(DiskCache::Writeback);
        let message = error(&vm, &ubuntu());
        assert!(
            message.contains(r#"`boot_disk.cache = "none"`"#),
            "{message}"
        );
    }

    #[test]
    fn vm_tuning_010_io_thread_cpus_without_virtio_disks_are_refused_naming_the_model_and_bus() {
        let mut image = ubuntu();
        let KindSettings::VirtualMachine(settings) = &mut image.settings else {
            panic!("ubuntu_linux is a VM");
        };
        settings.boot_disk.bus = DiskBuses::Sata;
        let mut vm = node(None);
        vm.boot_disk.io_thread_cpus = Some(vec![8]);
        let message = error(&vm, &image);
        assert!(message.contains("virtio disks"), "{message}");
        assert!(
            message.contains(&format!("'ubuntu_linux' uses {}", DiskBuses::Sata)),
            "{message}"
        );
    }

    #[test]
    fn vm_tuning_011_interface_queues_over_cpu_count_are_refused_giving_the_range() {
        let mut vm = node(Some(4));
        vm.interfaces.queues = Some(8);
        let message = error(&vm, &ubuntu());
        assert!(message.contains("between 1 and 4"), "{message}");
    }

    #[test]
    fn vm_tuning_012_interface_queues_without_virtio_interfaces_are_refused_naming_the_model() {
        let mut image = ubuntu();
        let KindSettings::VirtualMachine(settings) = &mut image.settings else {
            panic!("ubuntu_linux is a VM");
        };
        settings.interfaces.kind = InterfaceType::E1000;
        let mut vm = node(Some(8));
        vm.interfaces.queues = Some(8);
        let message = error(&vm, &image);
        assert!(message.contains("virtio interfaces"), "{message}");
        assert!(message.contains("'ubuntu_linux' uses E1000"), "{message}");
    }

    #[test]
    fn vm_tuning_013_two_nodes_pinning_a_vcpu_to_one_host_cpu_are_refused() {
        let message = format!(
            "{:#}",
            check_cpu_pin_overlap(&[pins("a", &[1, 2], &[]), pins("b", &[2, 3], &[])])
                .expect_err("overlap")
        );
        assert!(message.contains("Host CPU 2"), "{message}");
        assert!(
            message.contains("'a'") && message.contains("'b'"),
            "{message}"
        );
    }

    #[test]
    fn vm_tuning_014_qemu_threads_on_another_nodes_vcpu_cpu_are_refused() {
        let message = check_cpu_pin_overlap(&[pins("a", &[1, 2], &[]), pins("b", &[3], &[2])])
            .unwrap_err()
            .to_string();
        assert_eq!(
            message,
            "Host CPU 2 runs QEMU threads of node 'b' but is pinned to vCPUs of node 'a'"
        );
    }

    #[test]
    fn vm_tuning_015_the_qemu_threads_of_two_nodes_may_share_host_cpus() {
        check_cpu_pin_overlap(&[pins("a", &[1, 2], &[8]), pins("b", &[3, 4], &[8])]).unwrap();
    }

    #[test]
    fn vm_tuning_016_interface_queues_of_0_are_refused() {
        let mut vm = node(Some(4));
        vm.interfaces.queues = Some(0);
        let message = error(&vm, &ubuntu());
        assert!(message.contains("between 1 and 4"), "{message}");
    }
}
