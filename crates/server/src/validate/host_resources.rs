use anyhow::{Result, bail};

use topology::NodeExpanded;

use crate::services::error::ServiceError;

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
            bail!(ServiceError::HostCpuMissing {
                node: node_name.clone(),
                setting,
                cpu: *cpu,
                host_cpus,
            });
        }
    }
    if let Some(numa_node) = node.numa.node
        && u32::from(numa_node) >= host_numa_nodes
    {
        bail!(ServiceError::NumaNodeMissing {
            node: node_name.clone(),
            numa_node,
            host_numa_nodes,
        });
    }
    Ok(())
}

/// Check that a container's CPU quota, `cpu.count`, is no more than the server's CPUs, which
/// Docker refuses.
pub fn check_container_cpus(node_name: &str, cpu_count: u8, host_cpus: u32) -> Result<()> {
    if u32::from(cpu_count) > host_cpus {
        bail!(ServiceError::ContainerCpusOverHost {
            node: node_name.to_owned(),
            cpu_count,
            host_cpus,
        });
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

    fn error(node: &NodeExpanded, host_cpus: u32, host_numa_nodes: u32) -> ServiceError {
        check_host_resources(node, host_cpus, host_numa_nodes)
            .expect_err("tuning is rejected")
            .downcast::<ServiceError>()
            .expect("a service error")
    }

    fn missing_cpu(setting: &'static str, cpu: u16, host_cpus: u32) -> ServiceError {
        ServiceError::HostCpuMissing {
            node: "dev".to_owned(),
            setting,
            cpu,
            host_cpus,
        }
    }

    #[test]
    fn test_cpus_and_numa_node_on_the_host_are_accepted() {
        check_host_resources(&tuned(), 32, 2).unwrap();
        check_host_resources(&untuned(), 1, 1).unwrap();
    }

    #[test]
    fn test_each_cpu_list_must_name_host_cpus() {
        assert_eq!(error(&tuned(), 16, 2), missing_cpu("cpu.pinning", 25, 16));
        let mut emulator = untuned();
        emulator.cpu.emulator_pinning = Some(vec![40]);
        assert_eq!(
            error(&emulator, 32, 2),
            missing_cpu("cpu.emulator_pinning", 40, 32)
        );
        let mut io = untuned();
        io.boot_disk.io_thread_cpus = Some(vec![32]);
        assert_eq!(
            error(&io, 32, 2),
            missing_cpu("boot_disk.io_thread_cpus", 32, 32)
        );
    }

    #[test]
    fn test_numa_node_must_exist() {
        assert_eq!(
            error(&tuned(), 32, 1),
            ServiceError::NumaNodeMissing {
                node: "dev".to_owned(),
                numa_node: 1,
                host_numa_nodes: 1,
            }
        );
    }

    #[test]
    fn a_container_cpu_quota_up_to_the_servers_cpus_is_allowed() {
        check_container_cpus("db", 4, 4).unwrap();
        check_container_cpus("db", 1, 4).unwrap();
    }

    #[test]
    fn a_container_cpu_quota_over_the_servers_cpus_is_refused() {
        let error = check_container_cpus("db", 8, 4).unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::ContainerCpusOverHost {
                node: "db".to_owned(),
                cpu_count: 8,
                host_cpus: 4,
            })
        );
    }
}
