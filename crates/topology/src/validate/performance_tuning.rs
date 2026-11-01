use anyhow::{Result, bail};

use crate::NodeExpanded;
use shared::data::{
    DiskBuses, DiskCache, DiskIo, InterfaceType, NodeConfig, NodeCpuPins, NodeKind,
};

/// Check a node's `[nodes.performance_tuning]` against its own settings and its image.
pub fn check_performance_tuning(node: &NodeExpanded, image: &NodeConfig) -> Result<()> {
    let Some(tuning) = &node.performance_tuning else {
        return Ok(());
    };
    let name = &node.name;
    if image.kind != NodeKind::VirtualMachine {
        bail!(
            "Manifest - node: '{name}' sets `performance_tuning`, which is only for virtual \
             machines; '{}' is a {}",
            image.model,
            image.kind
        );
    }

    let needs_cpu_count = tuning.cpu_topology.is_some()
        || tuning.cpu_pinning.is_some()
        || tuning.interface_queues.is_some();
    let cpu_count = match node.cpu_count {
        Some(count) => u32::from(count),
        None if needs_cpu_count => bail!(
            "Manifest - node: '{name}' sets `cpu_topology`, `cpu_pinning` or \
             `interface_queues`; set `cpu_count` too"
        ),
        None => u32::from(image.cpu_count),
    };

    if let Some(topology) = &tuning.cpu_topology {
        let vcpus =
            u32::from(topology.sockets) * u32::from(topology.cores) * u32::from(topology.threads);
        if vcpus != cpu_count {
            bail!(
                "Manifest - node: '{name}' `cpu_topology` gives {vcpus} vCPUs, but `cpu_count` \
                 is {cpu_count}"
            );
        }
    }

    let vcpu_cpus = tuning.cpu_pinning.as_deref().unwrap_or_default();
    if let Some(pinning) = &tuning.cpu_pinning {
        if pinning.len() as u32 != cpu_count {
            bail!(
                "Manifest - node: '{name}' `cpu_pinning` lists {} host CPUs, one is needed for \
                 each of its {cpu_count} vCPUs",
                pinning.len()
            );
        }
        if let Some(cpu) = first_repeat(pinning) {
            bail!("Manifest - node: '{name}' `cpu_pinning` lists host CPU {cpu} more than once");
        }
    }

    for (setting, cpus) in [
        ("qemu_cpus", &tuning.qemu_cpus),
        ("disk_io_thread_cpus", &tuning.disk_io_thread_cpus),
    ] {
        let Some(cpus) = cpus else {
            continue;
        };
        if cpus.is_empty() {
            bail!("Manifest - node: '{name}' `{setting}` is empty; list host CPUs or remove it");
        }
        if let Some(cpu) = cpus.iter().find(|cpu| vcpu_cpus.contains(cpu)) {
            bail!(
                "Manifest - node: '{name}' `{setting}` uses host CPU {cpu}, which is pinned to \
                 one of its vCPUs"
            );
        }
    }

    if tuning.disk_io == Some(DiskIo::Native) && tuning.disk_cache != Some(DiskCache::None) {
        bail!(
            "Manifest - node: '{name}' sets `disk_io = \"native\"`, which needs \
             `disk_cache = \"none\"`"
        );
    }

    if tuning.disk_io_thread_cpus.is_some() && image.hdd_bus != DiskBuses::Virtio {
        bail!(
            "Manifest - node: '{name}' sets `disk_io_thread_cpus`, which needs virtio disks; \
             '{}' uses {}",
            image.model,
            image.hdd_bus
        );
    }

    if let Some(queues) = tuning.interface_queues {
        if queues == 0 || u32::from(queues) > cpu_count {
            bail!(
                "Manifest - node: '{name}' `interface_queues` must be between 1 and {cpu_count}, \
                 its `cpu_count`"
            );
        }
        if image.interface_type != InterfaceType::Virtio {
            bail!(
                "Manifest - node: '{name}' sets `interface_queues`, which needs virtio \
                 interfaces; '{}' uses {:?}",
                image.model,
                image.interface_type
            );
        }
    }

    Ok(())
}

/// Check that no host CPU pinned to a vCPU is shared: not with another node's vCPUs, and not
/// with any node's QEMU or I/O threads. QEMU and I/O threads may share CPUs with each other.
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
    use shared::data::{
        CpuTopology, DiskBuses, DiskCache, DiskIo, InterfaceType, NodeModel, PerformanceTuning,
    };

    fn node(cpu_count: Option<u8>, tuning: PerformanceTuning) -> NodeExpanded {
        NodeExpanded {
            name: "dev".to_string(),
            model: NodeModel::UbuntuLinux,
            cpu_count,
            performance_tuning: Some(tuning),
            ..NodeExpanded::default()
        }
    }

    fn ubuntu() -> NodeConfig {
        NodeConfig::get_model(NodeModel::UbuntuLinux)
    }

    fn error(node: &NodeExpanded, image: &NodeConfig) -> String {
        format!(
            "{:#}",
            check_performance_tuning(node, image).expect_err("tuning is rejected")
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
        let tuning = PerformanceTuning {
            cpu_topology: Some(CpuTopology {
                sockets: 1,
                cores: 2,
                threads: 2,
            }),
            cpu_pinning: Some(vec![9, 25, 10, 26]),
            qemu_cpus: Some(vec![8, 24]),
            numa_node: Some(1),
            disk_cache: Some(DiskCache::None),
            disk_io: Some(DiskIo::Native),
            disk_io_thread_cpus: Some(vec![8, 24]),
            interface_queues: Some(4),
            ..PerformanceTuning::default()
        };
        check_performance_tuning(&node(Some(4), tuning), &ubuntu()).unwrap();
    }

    #[test]
    fn test_node_without_tuning_is_accepted() {
        let mut node = node(None, PerformanceTuning::default());
        node.performance_tuning = None;
        check_performance_tuning(&node, &ubuntu()).unwrap();
    }

    #[test]
    fn test_tuning_is_only_for_virtual_machines() {
        let image = NodeConfig::get_model(NodeModel::FrrLinux);
        let message = error(&node(None, PerformanceTuning::default()), &image);
        assert!(message.contains("only for virtual machines"), "{message}");
    }

    #[test]
    fn test_topology_and_pinning_need_cpu_count() {
        let tuning = PerformanceTuning {
            cpu_pinning: Some(vec![1]),
            ..PerformanceTuning::default()
        };
        let message = error(&node(None, tuning), &ubuntu());
        assert!(message.contains("set `cpu_count`"), "{message}");
    }

    #[test]
    fn test_topology_must_match_cpu_count() {
        let tuning = PerformanceTuning {
            cpu_topology: Some(CpuTopology {
                sockets: 1,
                cores: 3,
                threads: 2,
            }),
            ..PerformanceTuning::default()
        };
        let message = error(&node(Some(4), tuning), &ubuntu());
        assert!(message.contains("6 vCPUs"), "{message}");
    }

    #[test]
    fn test_pinning_needs_one_cpu_per_vcpu_without_repeats() {
        let short = PerformanceTuning {
            cpu_pinning: Some(vec![1, 2]),
            ..PerformanceTuning::default()
        };
        assert!(error(&node(Some(4), short), &ubuntu()).contains("4 vCPUs"));
        let repeated = PerformanceTuning {
            cpu_pinning: Some(vec![1, 2, 2, 3]),
            ..PerformanceTuning::default()
        };
        assert!(error(&node(Some(4), repeated), &ubuntu()).contains("host CPU 2 more than once"));
    }

    #[test]
    fn test_qemu_threads_must_not_share_the_nodes_vcpu_cpus() {
        let tuning = PerformanceTuning {
            cpu_pinning: Some(vec![1, 2]),
            disk_io_thread_cpus: Some(vec![2]),
            ..PerformanceTuning::default()
        };
        let message = error(&node(Some(2), tuning), &ubuntu());
        assert!(message.contains("host CPU 2"), "{message}");
    }

    #[test]
    fn test_cpu_lists_must_not_be_empty() {
        let tuning = PerformanceTuning {
            qemu_cpus: Some(vec![]),
            ..PerformanceTuning::default()
        };
        let message = error(&node(None, tuning), &ubuntu());
        assert!(message.contains("`qemu_cpus` is empty"), "{message}");
    }

    #[test]
    fn test_native_io_needs_no_cache() {
        let tuning = PerformanceTuning {
            disk_io: Some(DiskIo::Native),
            disk_cache: Some(DiskCache::Writeback),
            ..PerformanceTuning::default()
        };
        let message = error(&node(None, tuning), &ubuntu());
        assert!(message.contains("`disk_cache = \"none\"`"), "{message}");
    }

    #[test]
    fn test_io_thread_needs_virtio_disks() {
        let mut image = ubuntu();
        image.hdd_bus = DiskBuses::Sata;
        let tuning = PerformanceTuning {
            disk_io_thread_cpus: Some(vec![8]),
            ..PerformanceTuning::default()
        };
        let message = error(&node(None, tuning), &image);
        assert!(message.contains("virtio disks"), "{message}");
    }

    #[test]
    fn test_interface_queues_need_virtio_and_at_most_cpu_count() {
        let tuning = PerformanceTuning {
            interface_queues: Some(8),
            ..PerformanceTuning::default()
        };
        let message = error(&node(Some(4), tuning.clone()), &ubuntu());
        assert!(message.contains("between 1 and 4"), "{message}");
        let mut image = ubuntu();
        image.interface_type = InterfaceType::E1000;
        let message = error(&node(Some(8), tuning), &image);
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
