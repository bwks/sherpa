use anyhow::{Result, bail};

use topology::NodeExpanded;

/// Check that the host CPUs and NUMA node a node's `cpu`, `boot_disk` and `numa` settings name
/// exist on the server. Host CPUs are numbered from 0, as libvirt numbers them.
pub fn check_host_resources(
    node: &NodeExpanded,
    host_cpus: u32,
    host_numa_nodes: u32,
) -> Result<()> {
    let node_name = &node.name;
    for (setting, cpus) in [
        ("cpu.pinning", &node.cpu.pinning),
        ("cpu.emulator_pinning", &node.cpu.emulator_pinning),
        ("boot_disk.io_thread_cpus", &node.boot_disk.io_thread_cpus),
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
    if let Some(numa_node) = node.numa.node
        && u32::from(numa_node) >= host_numa_nodes
    {
        bail!(
            "Node '{node_name}' `numa.node` is NUMA node {numa_node}, but the server has \
             {host_numa_nodes} NUMA node(s), numbered from 0"
        );
    }
    Ok(())
}

/// Check that a container's CPU quota, `cpu.count`, is no more than the server's CPUs, which
/// Docker refuses.
pub fn check_container_cpus(node_name: &str, cpu_count: u8, host_cpus: u32) -> Result<()> {
    if u32::from(cpu_count) > host_cpus {
        bail!(
            "Node '{node_name}' `cpu.count` is {cpu_count}, but the server has {host_cpus} CPUs; \
             a container's CPU quota cannot be more than the server's CPUs"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuned() -> NodeExpanded {
        let mut node = NodeExpanded {
            name: "dev".to_string(),
            ..NodeExpanded::default()
        };
        node.cpu.pinning = Some(vec![9, 25]);
        node.cpu.emulator_pinning = Some(vec![8]);
        node.boot_disk.io_thread_cpus = Some(vec![24]);
        node.numa.node = Some(1);
        node
    }

    fn untuned() -> NodeExpanded {
        NodeExpanded {
            name: "dev".to_string(),
            ..NodeExpanded::default()
        }
    }

    fn error(node: &NodeExpanded, host_cpus: u32, host_numa_nodes: u32) -> String {
        format!(
            "{:#}",
            check_host_resources(node, host_cpus, host_numa_nodes).expect_err("tuning is rejected")
        )
    }

    #[test]
    fn test_cpus_and_numa_node_on_the_host_are_accepted() {
        check_host_resources(&tuned(), 32, 2).unwrap();
        check_host_resources(&untuned(), 1, 1).unwrap();
    }

    #[test]
    fn test_each_cpu_list_must_name_host_cpus() {
        assert!(error(&tuned(), 16, 2).contains("`cpu.pinning` uses host CPU 25"));
        let mut emulator = untuned();
        emulator.cpu.emulator_pinning = Some(vec![40]);
        assert!(error(&emulator, 32, 2).contains("`cpu.emulator_pinning` uses host CPU 40"));
        let mut io = untuned();
        io.boot_disk.io_thread_cpus = Some(vec![32]);
        let message = error(&io, 32, 2);
        assert!(message.contains("CPUs 0 to 31"), "{message}");
    }

    #[test]
    fn test_numa_node_must_exist() {
        let message = error(&tuned(), 32, 1);
        assert!(message.contains("NUMA node 1"), "{message}");
    }

    #[test]
    fn a_container_cpu_quota_up_to_the_servers_cpus_is_allowed() {
        check_container_cpus("db", 4, 4).unwrap();
        check_container_cpus("db", 1, 4).unwrap();
    }

    #[test]
    fn a_container_cpu_quota_over_the_servers_cpus_is_refused() {
        let error = check_container_cpus("db", 8, 4).unwrap_err().to_string();
        assert!(error.contains("'db'"), "{error}");
        assert!(error.contains("`cpu.count` is 8"), "{error}");
        assert!(error.contains("4 CPUs"), "{error}");
    }
}
