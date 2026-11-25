use std::collections::HashMap;

use anyhow::{Context, Result, bail};

use crate::{BridgeDetailed, Connection, Link2, LinkDetailed, TopologyError};
use shared::data::NodeModel;
use shared::util::split_node_int;
use shared::validators::validate_link_impairment;

/// Which interface indices of a node can carry a link or a bridge.
struct InterfaceBounds<'a> {
    device_model: &'a NodeModel,
    data_interface_count: u8,
    reserved_interface_count: u8,
    first_data_interface_idx: u8,
    max_interface_idx: u8,
}

/// The interface index and name a link uses on `device_name`, if the link ends there.
fn link_end<'a>(link: &'a LinkDetailed, device_name: &str) -> Option<(u8, &'a str)> {
    if link.node_a == device_name {
        Some((link.int_a_idx, link.int_a.as_str()))
    } else if link.node_b == device_name {
        Some((link.int_b_idx, link.int_b.as_str()))
    } else {
        None
    }
}

/// Check for duplicate interface usage across device links and bridges.
/// Each interface on a device can only be used once (either in a link OR a bridge).
///
/// # Errors
///
/// Fails when an interface is used by more than one link or bridge.
pub fn check_duplicate_interface_link(
    links: &[LinkDetailed],
    bridges: &[BridgeDetailed],
) -> Result<()> {
    let mut device_int_map: HashMap<String, Vec<u8>> = HashMap::new();

    for link in links {
        check_device_interface(
            &link.node_a,
            link.int_a_idx,
            &mut device_int_map,
            &Connection::Link,
        )?;
        check_device_interface(
            &link.node_b,
            link.int_b_idx,
            &mut device_int_map,
            &Connection::Link,
        )?;
    }

    for bridge in bridges {
        let connection = Connection::Bridge {
            name: bridge.manifest_name.clone(),
        };
        for bridge_link in &bridge.links {
            check_device_interface(
                &bridge_link.node_name,
                bridge_link.interface_index,
                &mut device_int_map,
                &connection,
            )?;
        }
    }

    Ok(())
}

/// Record that `device` uses `interface`, failing if something already does.
fn check_device_interface(
    device: &str,
    interface: u8,
    device_int_map: &mut HashMap<String, Vec<u8>>,
    connection: &Connection,
) -> Result<()> {
    let interfaces = device_int_map.entry(device.to_string()).or_default();
    if interfaces.contains(&interface) {
        bail!(TopologyError::InterfaceInUse {
            connection: connection.clone(),
            node: device.to_owned(),
            index: interface,
        });
    }
    interfaces.push(interface);
    Ok(())
}

/// Check no link has both ends on one node. An end not written `node::interface` is left to
/// the link resolution, which refuses it.
///
/// # Errors
///
/// Fails on the first link whose two ends name the same node.
pub fn check_links_to_itself(links: &[Link2]) -> Result<()> {
    for link in links {
        if let (Ok((node_a, _)), Ok((node_b, _))) =
            (split_node_int(&link.src), split_node_int(&link.dst))
            && node_a == node_b
        {
            bail!(TopologyError::LinkToItself {
                src: link.src.clone(),
                dst: link.dst.clone(),
                node: node_a,
            });
        }
    }
    Ok(())
}

/// Check every link's impairment against what netem can apply.
///
/// # Errors
///
/// Fails when a link's delay or jitter is longer than netem can hold, or a percentage is not
/// between 0 and 100.
pub fn check_link_impairments(links: &[Link2]) -> Result<()> {
    for link in links {
        let Some(impairment) = &link.impairment else {
            continue;
        };
        validate_link_impairment(
            impairment.delay.unwrap_or(0),
            impairment.jitter.unwrap_or(0),
            impairment.loss_percent.unwrap_or(0.0),
            impairment.reorder_percent.unwrap_or(0.0),
            impairment.corrupt_percent.unwrap_or(0.0),
        )
        .with_context(|| TopologyError::LinkImpairment {
            src: link.src.clone(),
            dst: link.dst.clone(),
        })?;
    }
    Ok(())
}

/// Check interface index bounds for both links and bridges.
/// Interfaces defined in connections should not be:
/// - the management interface, index 0 on every model
/// - reserved interfaces
/// - greater than the maximum interface index calculated from `interfaces.data_count`
///
/// # Errors
///
/// Fails when a link or bridge uses an interface outside the node's data interfaces.
pub fn check_interface_bounds(
    device_name: &str,
    device_model: &NodeModel,
    data_interface_count: u8,
    reserved_interface_count: u8,
    links: &[LinkDetailed],
    bridges: &[BridgeDetailed],
) -> Result<()> {
    // Management is always at index 0, reserved interfaces at 1 to reserved_interface_count,
    // and data interfaces after them. With data_interface_count=52 and none reserved, data
    // interfaces are 1 to 52; with no data interfaces, the maximum is the last reserved one.
    let first_data_interface_idx = reserved_interface_count.checked_add(1).with_context(|| {
        format!(
            "Interface bounds validation overflow for node '{device_name}' model '{device_model}' (reserved: {reserved_interface_count})"
        )
    })?;
    let max_interface_idx = reserved_interface_count
        .checked_add(data_interface_count)
        .with_context(|| {
            format!(
                "Interface bounds validation overflow for node '{device_name}' model '{device_model}' (data_interface_count: {data_interface_count}, reserved: {reserved_interface_count})"
            )
        })?;
    let bounds = InterfaceBounds {
        device_model,
        data_interface_count,
        reserved_interface_count,
        first_data_interface_idx,
        max_interface_idx,
    };

    for link in links {
        let Some((interface_idx, interface_name)) = link_end(link, device_name) else {
            continue;
        };
        check_interface_index(
            &Connection::Link,
            device_name,
            interface_name,
            interface_idx,
            &bounds,
        )?;
    }

    for bridge in bridges {
        let connection = Connection::Bridge {
            name: bridge.manifest_name.clone(),
        };
        for bridge_link in &bridge.links {
            if bridge_link.node_name != device_name {
                continue;
            }
            check_interface_index(
                &connection,
                device_name,
                &bridge_link.interface_name,
                bridge_link.interface_index,
                &bounds,
            )?;
        }
    }

    Ok(())
}

/// Check that `interface_idx`, the interface `interface` of `node` in `connection`, is a data
/// interface.
fn check_interface_index(
    connection: &Connection,
    node: &str,
    interface: &str,
    interface_idx: u8,
    bounds: &InterfaceBounds,
) -> Result<()> {
    if interface_idx == 0 {
        bail!(TopologyError::ManagementInterface {
            connection: connection.clone(),
            node: node.to_owned(),
            interface: interface.to_owned(),
        });
    } else if interface_idx < bounds.first_data_interface_idx {
        bail!(TopologyError::ReservedInterface {
            connection: connection.clone(),
            node: node.to_owned(),
            interface: interface.to_owned(),
            reserved: bounds.reserved_interface_count,
        });
    } else if interface_idx > bounds.max_interface_idx {
        bail!(TopologyError::InterfacePastLast {
            connection: connection.clone(),
            node: node.to_owned(),
            interface: interface.to_owned(),
            max_index: bounds.max_interface_idx,
            model: *bounds.device_model,
            data_interface_count: bounds.data_interface_count,
            reserved: bounds.reserved_interface_count,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BridgeLinkDetailed;
    use crate::link::ManifestImpairment;
    use shared::data::ModelKind;

    /// Helper to create a test link
    fn create_link(node_a: &str, int_a: u8, node_b: &str, int_b: u8) -> LinkDetailed {
        LinkDetailed {
            node_a: node_a.to_string(),
            node_a_idx: 0,
            node_a_model_kind: ModelKind::default_for(NodeModel::RockyLinux),
            int_a: "test".to_string(),
            int_a_idx: int_a,
            node_b: node_b.to_string(),
            node_b_idx: 1,
            node_b_model_kind: ModelKind::default_for(NodeModel::RockyLinux),
            int_b: "test".to_string(),
            int_b_idx: int_b,
            link_idx: 0,
            impairment: None,
        }
    }

    #[test]
    fn manifest_links_006_the_management_interface_is_refused_in_a_link_and_in_a_bridge() {
        let links = vec![create_link("rocky1", 0, "rocky2", 1)];
        let error = check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, &links, &[])
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::ManagementInterface {
                connection: Connection::Link,
                node: "rocky1".to_owned(),
                interface: "test".to_owned(),
            })
        );

        let bridges = vec![create_bridge("br1", vec![("rocky1", 0), ("rocky2", 5)])];
        let error = check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, &[], &bridges)
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::ManagementInterface {
                connection: bridge_br1(),
                node: "rocky1".to_owned(),
                interface: "eth0".to_owned(),
            })
        );
    }

    #[test]
    fn manifest_links_007_a_reserved_interface_is_refused_and_the_first_after_them_is_taken() {
        // Cisco IOS XRv9000 reserves indices 1 and 2; 3 is its first data interface.
        let links = vec![create_link("xr1", 2, "xr2", 3)];
        let error = check_interface_bounds("xr1", &NodeModel::CiscoIosxr9000, 31, 2, &links, &[])
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::ReservedInterface {
                connection: Connection::Link,
                node: "xr1".to_owned(),
                interface: "test".to_owned(),
                reserved: 2,
            })
        );

        let links = vec![create_link("xr1", 3, "xr2", 3)];
        check_interface_bounds("xr1", &NodeModel::CiscoIosxr9000, 31, 2, &links, &[]).unwrap();
    }

    #[test]
    fn manifest_links_008_an_interface_past_the_last_data_interface_is_refused() {
        let past_last =
            |connection: Connection, interface: &str| TopologyError::InterfacePastLast {
                connection,
                node: "rocky1".to_owned(),
                interface: interface.to_owned(),
                max_index: 52,
                model: NodeModel::RockyLinux,
                data_interface_count: 52,
                reserved: 0,
            };
        let links = vec![create_link("rocky1", 53, "rocky2", 1)];
        let error = check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, &links, &[])
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&past_last(Connection::Link, "test"))
        );

        let bridges = vec![create_bridge("br1", vec![("rocky1", 100)])];
        let error = check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, &[], &bridges)
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&past_last(bridge_br1(), "eth100"))
        );

        // Cisco IOS XRv9000: the last data interface is 3 + 31 - 1 = 33.
        let links = vec![create_link("xr1", 33, "xr2", 3)];
        check_interface_bounds("xr1", &NodeModel::CiscoIosxr9000, 31, 2, &links, &[]).unwrap();
        let bridges = vec![create_bridge("br1", vec![("rocky1", 52)])];
        check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, &[], &bridges).unwrap();
    }

    fn bridge_br1() -> Connection {
        Connection::Bridge {
            name: "br1".to_owned(),
        }
    }

    /// Helper to create a test bridge with connections
    fn create_bridge(bridge_name: &str, connections: Vec<(&str, u8)>) -> BridgeDetailed {
        BridgeDetailed {
            manifest_name: bridge_name.to_string(), // Use bridge_name as manifest_name for testing
            bridge_name: bridge_name.to_string(),
            libvirt_name: format!("test_{bridge_name}"),
            index: 0,
            links: connections
                .into_iter()
                .map(|(node_name, interface_index)| BridgeLinkDetailed {
                    node_name: node_name.to_string(),
                    node_model_kind: ModelKind::default_for(NodeModel::RockyLinux),
                    interface_name: format!("eth{interface_index}"),
                    interface_index,
                })
                .collect(),
        }
    }

    #[test]
    fn manifest_links_009_an_interface_used_twice_is_refused_naming_the_node_and_the_index() {
        for (links, bridges, connection, index) in [
            (
                vec![
                    create_link("rocky1", 7, "rocky2", 1),
                    create_link("rocky1", 7, "rocky3", 1),
                ],
                vec![],
                Connection::Link,
                7,
            ),
            (
                vec![create_link("rocky1", 5, "rocky2", 10)],
                vec![create_bridge("br1", vec![("rocky1", 5), ("rocky3", 15)])],
                bridge_br1(),
                5,
            ),
            (
                vec![],
                vec![
                    create_bridge("br1", vec![("rocky1", 10), ("rocky2", 15)]),
                    create_bridge("br2", vec![("rocky1", 10), ("rocky3", 20)]),
                ],
                Connection::Bridge {
                    name: "br2".to_owned(),
                },
                10,
            ),
        ] {
            let error = check_duplicate_interface_link(&links, &bridges).unwrap_err();
            assert_eq!(
                error.downcast_ref::<TopologyError>(),
                Some(&TopologyError::InterfaceInUse {
                    connection,
                    node: "rocky1".to_owned(),
                    index,
                })
            );
        }
    }

    fn plain_link(src: &str, dst: &str) -> Link2 {
        Link2 {
            src: src.to_owned(),
            dst: dst.to_owned(),
            impairment: None,
        }
    }

    #[test]
    fn manifest_links_015_a_link_from_a_node_to_itself_is_refused_naming_the_link_and_the_node() {
        for dst in ["r1::eth2", "r1::eth1"] {
            let error = check_links_to_itself(&[
                plain_link("r1::eth3", "r2::eth3"),
                plain_link("r1::eth1", dst),
            ])
            .unwrap_err();
            assert_eq!(
                error.downcast_ref::<TopologyError>(),
                Some(&TopologyError::LinkToItself {
                    src: "r1::eth1".to_owned(),
                    dst: dst.to_owned(),
                    node: "r1".to_owned(),
                })
            );
        }
        assert!(check_links_to_itself(&[plain_link("r1::eth1", "r2::eth1")]).is_ok());
    }

    fn impaired_link(impairment: ManifestImpairment) -> Link2 {
        Link2 {
            src: "dev01::eth1".to_string(),
            dst: "dev02::eth1".to_string(),
            impairment: Some(impairment),
        }
    }

    #[test]
    fn manifest_links_013_an_impairment_netem_cannot_apply_is_refused_naming_the_link_and_field() {
        let percent = |field: &str, value: f64| {
            let mut impairment = ManifestImpairment::default();
            match field {
                "loss_percent" => impairment.loss_percent = Some(value),
                "reorder_percent" => impairment.reorder_percent = Some(value),
                _ => impairment.corrupt_percent = Some(value),
            }
            impairment
        };
        let mut refused = vec![
            (
                "delay",
                ManifestImpairment {
                    delay: Some(274_878),
                    ..Default::default()
                },
            ),
            (
                "jitter",
                ManifestImpairment {
                    jitter: Some(274_878),
                    ..Default::default()
                },
            ),
        ];
        for field in ["loss_percent", "reorder_percent", "corrupt_percent"] {
            refused.push((field, percent(field, 100.5)));
            refused.push((field, percent(field, -1.0)));
        }
        for (field, impairment) in refused {
            let error = check_link_impairments(&[impaired_link(impairment)]).unwrap_err();
            assert_eq!(
                error.downcast_ref::<TopologyError>(),
                Some(&TopologyError::LinkImpairment {
                    src: "dev01::eth1".to_owned(),
                    dst: "dev02::eth1".to_owned(),
                }),
                "{field}"
            );
        }
        check_link_impairments(&[impaired_link(ManifestImpairment {
            delay: Some(274_877),
            jitter: Some(274_877),
            loss_percent: Some(100.0),
            reorder_percent: Some(0.0),
            corrupt_percent: Some(1.5),
        })])
        .unwrap();
    }
}
