use anyhow::{Result, bail};

use shared::data::PerformanceTuning;

/// Check that the host CPUs and NUMA node a node's tuning names exist on the server. Host CPUs
/// are numbered from 0, as libvirt numbers them.
pub fn check_host_resources(
    node_name: &str,
    tuning: &PerformanceTuning,
    host_cpus: u32,
    host_numa_nodes: u32,
) -> Result<()> {
    for (setting, cpus) in [
        ("cpu_pinning", &tuning.cpu_pinning),
        ("qemu_cpus", &tuning.qemu_cpus),
        ("disk_io_thread_cpus", &tuning.disk_io_thread_cpus),
    ] {
        let Some(cpus) = cpus else {
            continue;
        };
        if let Some(cpu) = cpus.iter().find(|cpu| u32::from(**cpu) >= host_cpus) {
            bail!(
                "Node '{node_name}' `{setting}` uses host CPU {cpu}, but the server has CPUs 0 \
                 to {}",
                host_cpus.saturating_sub(1)
            );
        }
    }
    if let Some(node) = tuning.numa_node
        && u32::from(node) >= host_numa_nodes
    {
        bail!(
            "Node '{node_name}' `numa_node` is NUMA node {node}, but the server has {host_numa_nodes} \
             NUMA node(s), numbered from 0"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning() -> PerformanceTuning {
        PerformanceTuning {
            cpu_pinning: Some(vec![9, 25]),
            qemu_cpus: Some(vec![8]),
            disk_io_thread_cpus: Some(vec![24]),
            numa_node: Some(1),
            ..PerformanceTuning::default()
        }
    }

    fn error(tuning: &PerformanceTuning, host_cpus: u32, host_numa_nodes: u32) -> String {
        format!(
            "{:#}",
            check_host_resources("dev", tuning, host_cpus, host_numa_nodes)
                .expect_err("tuning is rejected")
        )
    }

    #[test]
    fn test_cpus_and_numa_node_on_the_host_are_accepted() {
        check_host_resources("dev", &tuning(), 32, 2).unwrap();
        check_host_resources("dev", &PerformanceTuning::default(), 1, 1).unwrap();
    }

    #[test]
    fn test_each_cpu_list_must_name_host_cpus() {
        assert!(error(&tuning(), 16, 2).contains("`cpu_pinning` uses host CPU 25"));
        let qemu = PerformanceTuning {
            qemu_cpus: Some(vec![40]),
            ..PerformanceTuning::default()
        };
        assert!(error(&qemu, 32, 2).contains("`qemu_cpus` uses host CPU 40"));
        let io = PerformanceTuning {
            disk_io_thread_cpus: Some(vec![32]),
            ..PerformanceTuning::default()
        };
        let message = error(&io, 32, 2);
        assert!(message.contains("CPUs 0 to 31"), "{message}");
    }

    #[test]
    fn test_numa_node_must_exist() {
        let message = error(&tuning(), 32, 1);
        assert!(message.contains("NUMA node 1"), "{message}");
    }
}
