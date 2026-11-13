use anyhow::{Result, bail};

use crate::NodeExpanded;
use shared::data::{
    DiskBuses, DiskCache, DiskIo, InterfaceType, NodeConfig, NodeCpuPins, NodeKind,
};

/// Check a VM's CPU, NUMA, boot disk and interface tuning against its own settings and its
/// image. Other kinds do not take these settings, which `check_node_kind_settings` refuses.
///
/// # Errors
///
/// Fails when a setting does not fit the node's CPU count, disks or interfaces.
pub fn check_vm_tuning(node: &NodeExpanded, image: &NodeConfig) -> Result<()> {
    if image.kind != NodeKind::VirtualMachine {
        return Ok(());
    }
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

    if disk.io_thread_cpus.is_some() && image.boot_disk.bus != DiskBuses::Virtio {
        bail!(
            "Manifest - node: '{name}' sets `boot_disk.io_thread_cpus`, which needs virtio \
             disks; '{}' uses {}",
            image.model,
            image.boot_disk.bus
        );
    }

    if let Some(queues) = node.interfaces.queues {
        if queues == 0 || u32::from(queues) > cpu_count {
            bail!(
                "Manifest - node: '{name}' `interfaces.queues` must be between 1 and \
                 {cpu_count}, its `cpu.count`"
            );
        }
        if image.interfaces.kind != InterfaceType::Virtio {
            bail!(
                "Manifest - node: '{name}' sets `interfaces.queues`, which needs virtio \
                 interfaces; '{}' uses {:?}",
                image.model,
                image.interfaces.kind
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
    use shared::data::{CpuTopology, NodeCpu, NodeModel, NumaMemoryMode};

    fn node(cpu_count: Option<u8>) -> NodeExpanded {
        NodeExpanded {
            name: "dev".to_string(),
            model: NodeModel::UbuntuLinux,
            cpu: NodeCpu {
                count: cpu_count,
                ..NodeCpu::default()
            },
            ..NodeExpanded::default()
        }
    }

    fn ubuntu() -> NodeConfig {
        NodeConfig::get_model(NodeModel::UbuntuLinux)
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
    fn test_full_tuning_on_a_vm_is_accepted() {
        let mut vm = node(Some(4));
        vm.cpu.topology = Some(CpuTopology {
            sockets: 1,
            cores: 2,
            threads: 2,
        });
        vm.cpu.pinning = Some(vec![9, 25, 10, 26]);
        vm.cpu.emulator_pinning = Some(vec![8, 24]);
        vm.numa.node = Some(1);
        vm.boot_disk.cache = Some(DiskCache::None);
        vm.boot_disk.io = Some(DiskIo::Native);
        vm.boot_disk.io_thread_cpus = Some(vec![8, 24]);
        vm.interfaces.queues = Some(4);
        check_vm_tuning(&vm, &ubuntu()).unwrap();
    }

    #[test]
    fn test_node_without_tuning_is_accepted() {
        check_vm_tuning(&node(None), &ubuntu()).unwrap();
    }

    #[test]
    fn test_topology_and_pinning_need_cpu_count() {
        let mut vm = node(None);
        vm.cpu.pinning = Some(vec![1]);
        let message = error(&vm, &ubuntu());
        assert!(message.contains("set `cpu.count`"), "{message}");
    }

    #[test]
    fn test_topology_must_match_cpu_count() {
        let mut vm = node(Some(4));
        vm.cpu.topology = Some(CpuTopology {
            sockets: 1,
            cores: 3,
            threads: 2,
        });
        let message = error(&vm, &ubuntu());
        assert!(message.contains("6 vCPUs"), "{message}");
    }

    #[test]
    fn test_pinning_needs_one_cpu_per_vcpu_without_repeats() {
        let mut short = node(Some(4));
        short.cpu.pinning = Some(vec![1, 2]);
        assert!(error(&short, &ubuntu()).contains("4 vCPUs"));
        let mut repeated = node(Some(4));
        repeated.cpu.pinning = Some(vec![1, 2, 2, 3]);
        assert!(error(&repeated, &ubuntu()).contains("host CPU 2 more than once"));
    }

    #[test]
    fn test_qemu_threads_must_not_share_the_nodes_vcpu_cpus() {
        let mut vm = node(Some(2));
        vm.cpu.pinning = Some(vec![1, 2]);
        vm.boot_disk.io_thread_cpus = Some(vec![2]);
        let message = error(&vm, &ubuntu());
        assert!(message.contains("host CPU 2"), "{message}");
    }

    #[test]
    fn test_cpu_lists_must_not_be_empty() {
        let mut vm = node(None);
        vm.cpu.emulator_pinning = Some(vec![]);
        let message = error(&vm, &ubuntu());
        assert!(
            message.contains("`cpu.emulator_pinning` is empty"),
            "{message}"
        );
    }

    #[test]
    fn test_numa_memory_mode_needs_numa_node() {
        let mut vm = node(None);
        vm.numa.memory_mode = Some(NumaMemoryMode::Preferred);
        let message = error(&vm, &ubuntu());
        assert!(
            message.contains("sets `numa.memory_mode` without `numa.node`"),
            "{message}"
        );
    }

    #[test]
    fn test_numa_memory_mode_with_numa_node_is_accepted() {
        let mut vm = node(None);
        vm.numa.node = Some(0);
        vm.numa.memory_mode = Some(NumaMemoryMode::Preferred);
        check_vm_tuning(&vm, &ubuntu()).unwrap();
    }

    #[test]
    fn test_native_io_needs_no_cache() {
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
    fn test_io_thread_needs_virtio_disks() {
        let mut image = ubuntu();
        image.boot_disk.bus = DiskBuses::Sata;
        let mut vm = node(None);
        vm.boot_disk.io_thread_cpus = Some(vec![8]);
        let message = error(&vm, &image);
        assert!(message.contains("virtio disks"), "{message}");
    }

    #[test]
    fn test_interface_queues_need_virtio_and_at_most_cpu_count() {
        let mut vm = node(Some(4));
        vm.interfaces.queues = Some(8);
        let message = error(&vm, &ubuntu());
        assert!(message.contains("between 1 and 4"), "{message}");
        let mut image = ubuntu();
        image.interfaces.kind = InterfaceType::E1000;
        let mut vm = node(Some(8));
        vm.interfaces.queues = Some(8);
        let message = error(&vm, &image);
        assert!(message.contains("virtio interfaces"), "{message}");
    }

    #[test]
    fn test_vcpu_cpus_are_exclusive_between_nodes() {
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
    fn test_qemu_threads_must_not_use_another_nodes_vcpu_cpus() {
        let result = check_cpu_pin_overlap(&[pins("a", &[1, 2], &[]), pins("b", &[3], &[2])]);
        assert!(result.is_err());
    }

    #[test]
    fn test_qemu_threads_may_share_cpus() {
        check_cpu_pin_overlap(&[pins("a", &[1, 2], &[8]), pins("b", &[3, 4], &[8])]).unwrap();
    }
}
