use anyhow::{Result, bail};

use crate::{Node, TopologyError};

/// Check no node sets `memory`, `cpu.count`, `boot_disk.size`, `interfaces.data_count` or
/// `ready_check.port` to 0. The types bound each from above; none of them takes 0, which Docker
/// would read as no limit and libvirt and QEMU refuse late.
///
/// # Errors
///
/// Fails on the first node that sets one to 0, naming the node and the field.
pub fn check_nonzero_node_settings(nodes: &[Node]) -> Result<()> {
    for node in nodes {
        let settings = [
            ("memory", node.memory.map(u32::from)),
            ("cpu.count", node.cpu.count.map(u32::from)),
            ("boot_disk.size", node.boot_disk.size.map(u32::from)),
            (
                "interfaces.data_count",
                node.interfaces.data_count.map(u32::from),
            ),
            ("ready_check.port", node.ready_check.port.map(u32::from)),
        ];
        if let Some((field, _)) = settings.iter().find(|(_, value)| *value == Some(0)) {
            bail!(TopologyError::ZeroSetting {
                node: node.name.clone(),
                field,
            });
        }
    }
    Ok(())
}

/// Check the lab's `ready_timeout` is from 1 second to the most the lab's definition stores.
///
/// # Errors
///
/// Fails when it is 0 or larger than `i64::MAX` seconds.
pub fn check_ready_timeout(ready_timeout: Option<u64>) -> Result<()> {
    match ready_timeout {
        Some(0) => bail!(TopologyError::ReadyTimeoutZero),
        Some(seconds) if i64::try_from(seconds).is_err() => {
            bail!(TopologyError::ReadyTimeoutTooLong { seconds })
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeReadyCheck;
    use shared::data::{NodeBootDisk, NodeCpu, NodeInterfaces};

    #[test]
    fn node_settings_014_zero_is_refused_for_memory_cpus_disk_size_data_interfaces_and_port() {
        let memory = Node {
            memory: Some(0),
            ..Default::default()
        };
        let cpus = Node {
            cpu: NodeCpu {
                count: Some(0),
                ..Default::default()
            },
            ..Default::default()
        };
        let disk = Node {
            boot_disk: NodeBootDisk {
                size: Some(0),
                ..Default::default()
            },
            ..Default::default()
        };
        let interfaces = Node {
            interfaces: NodeInterfaces {
                data_count: Some(0),
                ..Default::default()
            },
            ..Default::default()
        };
        let port = Node {
            ready_check: NodeReadyCheck {
                port: Some(0),
                ..Default::default()
            },
            ..Default::default()
        };
        for (field, mut node) in [
            ("memory", memory),
            ("cpu.count", cpus),
            ("boot_disk.size", disk),
            ("interfaces.data_count", interfaces),
            ("ready_check.port", port),
        ] {
            node.name = "r1".to_owned();
            let error = check_nonzero_node_settings(&[Node::default(), node])
                .expect_err(&format!("{field} = 0 should be refused"));
            assert_eq!(
                error.downcast_ref::<TopologyError>(),
                Some(&TopologyError::ZeroSetting {
                    node: "r1".to_owned(),
                    field,
                })
            );
        }

        let one = Node {
            memory: Some(1),
            cpu: NodeCpu {
                count: Some(1),
                ..Default::default()
            },
            boot_disk: NodeBootDisk {
                size: Some(1),
                ..Default::default()
            },
            interfaces: NodeInterfaces {
                data_count: Some(1),
                ..Default::default()
            },
            ready_check: NodeReadyCheck {
                port: Some(1),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(check_nonzero_node_settings(&[one]).is_ok());
    }

    #[test]
    fn lab_settings_008_a_ready_timeout_of_0_is_refused_and_1_is_taken() {
        let error = check_ready_timeout(Some(0)).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::ReadyTimeoutZero)
        );
        assert!(check_ready_timeout(Some(1)).is_ok());
        assert!(check_ready_timeout(None).is_ok());
    }

    #[test]
    fn lab_settings_011_a_ready_timeout_over_the_stored_maximum_is_refused_and_the_maximum_is_taken()
     {
        let error = check_ready_timeout(Some(9_223_372_036_854_775_808)).unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::ReadyTimeoutTooLong {
                seconds: 9_223_372_036_854_775_808,
            })
        );
        assert!(check_ready_timeout(Some(9_223_372_036_854_775_807)).is_ok());
    }
}
