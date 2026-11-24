use std::collections::HashMap;

use anyhow::{Context, Result, bail};

use crate::{BridgeDetailed, Link2, LinkDetailed};
use shared::data::NodeModel;
use shared::validators::validate_link_impairment;

/// Which interface indices of a node can carry a link or a bridge.
struct InterfaceBounds<'a> {
    device_model: &'a NodeModel,
    data_interface_count: u8,
    reserved_interface_count: u8,
    /// The first data interface, after the management interface and the reserved ones.
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
        check_device_interface(&link.node_a, link.int_a_idx, &mut device_int_map, "link")?;
        check_device_interface(&link.node_b, link.int_b_idx, &mut device_int_map, "link")?;
    }

    for bridge in bridges {
        for bridge_link in &bridge.links {
            check_device_interface(
                &bridge_link.node_name,
                bridge_link.interface_index,
                &mut device_int_map,
                &format!("bridge '{}'", bridge.manifest_name),
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
    connection_type: &str,
) -> Result<()> {
    let interfaces = device_int_map.entry(device.to_string()).or_default();
    if interfaces.contains(&interface) {
        bail!(
            "Manifest {connection_type} - node '{device}' interface index '{interface}' is already in use"
        );
    }
    interfaces.push(interface);
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
        .with_context(|| format!("Manifest link - {} <-> {}", link.src, link.dst))?;
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
            &format!("Manifest link - node '{device_name}' interface '{interface_name}'"),
            "links",
            interface_idx,
            &bounds,
        )?;
    }

    for bridge in bridges {
        for bridge_link in &bridge.links {
            if bridge_link.node_name != device_name {
                continue;
            }
            check_interface_index(
                &format!(
                    "Manifest bridge '{}' - node '{device_name}' interface '{}'",
                    bridge.manifest_name, bridge_link.interface_name
                ),
                "bridges",
                bridge_link.interface_index,
                &bounds,
            )?;
        }
    }

    Ok(())
}

/// Check that `interface_idx` is a data interface. `connection` names the interface and what
/// uses it, and `used_in` the kind of connection, for the error.
fn check_interface_index(
    connection: &str,
    used_in: &str,
    interface_idx: u8,
    bounds: &InterfaceBounds,
) -> Result<()> {
    if interface_idx == 0 {
        bail!("{connection} is the management interface and cannot be used in {used_in}");
    } else if interface_idx < bounds.first_data_interface_idx {
        bail!(
            "{connection} is a reserved interface (reserved_count: {}) and cannot be used in {used_in}",
            bounds.reserved_interface_count
        );
    } else if interface_idx > bounds.max_interface_idx {
        bail!(
            "{connection} exceeds the maximum interface index '{}' for node model '{}' (data_interface_count: {}, reserved: {})",
            bounds.max_interface_idx,
            bounds.device_model,
            bounds.data_interface_count,
            bounds.reserved_interface_count
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BridgeLinkDetailed;
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
        let err_msg = check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, &links, &[])
            .unwrap_err()
            .to_string();
        assert!(
            err_msg.contains("is the management interface and cannot be used in links"),
            "{err_msg}"
        );

        let bridges = vec![create_bridge("br1", vec![("rocky1", 0), ("rocky2", 5)])];
        let err_msg =
            check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, &[], &bridges)
                .unwrap_err()
                .to_string();
        assert!(err_msg.contains("bridge 'br1'"), "{err_msg}");
        assert!(
            err_msg.contains("is the management interface and cannot be used in bridges"),
            "{err_msg}"
        );
    }

    #[test]
    fn manifest_links_007_a_reserved_interface_is_refused_and_the_first_after_them_is_taken() {
        // Cisco IOS XRv9000 reserves indices 1 and 2; 3 is its first data interface.
        let links = vec![create_link("xr1", 2, "xr2", 3)];
        let err_msg = check_interface_bounds("xr1", &NodeModel::CiscoIosxr9000, 31, 2, &links, &[])
            .unwrap_err()
            .to_string();
        assert!(err_msg.contains("reserved interface"), "{err_msg}");

        let links = vec![create_link("xr1", 3, "xr2", 3)];
        check_interface_bounds("xr1", &NodeModel::CiscoIosxr9000, 31, 2, &links, &[]).unwrap();
    }

    #[test]
    fn manifest_links_008_an_interface_past_the_last_data_interface_is_refused() {
        let links = vec![create_link("rocky1", 53, "rocky2", 1)];
        let err_msg = check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, &links, &[])
            .unwrap_err()
            .to_string();
        assert!(
            err_msg.contains("exceeds the maximum interface index '52'"),
            "{err_msg}"
        );

        let bridges = vec![create_bridge("br1", vec![("rocky1", 100)])];
        let err_msg =
            check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, &[], &bridges)
                .unwrap_err()
                .to_string();
        assert!(err_msg.contains("bridge 'br1'"), "{err_msg}");
        assert!(
            err_msg.contains("exceeds the maximum interface index '52'"),
            "{err_msg}"
        );

        // Cisco IOS XRv9000: the last data interface is 3 + 31 - 1 = 33.
        let links = vec![create_link("xr1", 33, "xr2", 3)];
        check_interface_bounds("xr1", &NodeModel::CiscoIosxr9000, 31, 2, &links, &[]).unwrap();
        let bridges = vec![create_bridge("br1", vec![("rocky1", 52)])];
        check_interface_bounds("rocky1", &NodeModel::RockyLinux, 52, 0, &[], &bridges).unwrap();
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
        for (links, bridges, index) in [
            (
                vec![
                    create_link("rocky1", 7, "rocky2", 1),
                    create_link("rocky1", 7, "rocky3", 1),
                ],
                vec![],
                7,
            ),
            (
                vec![create_link("rocky1", 5, "rocky2", 10)],
                vec![create_bridge("br1", vec![("rocky1", 5), ("rocky3", 15)])],
                5,
            ),
            (
                vec![],
                vec![
                    create_bridge("br1", vec![("rocky1", 10), ("rocky2", 15)]),
                    create_bridge("br2", vec![("rocky1", 10), ("rocky3", 20)]),
                ],
                10,
            ),
        ] {
            let err_msg = check_duplicate_interface_link(&links, &bridges)
                .unwrap_err()
                .to_string();
            assert!(
                err_msg.contains(&format!(
                    "node 'rocky1' interface index '{index}' is already in use"
                )),
                "{err_msg}"
            );
        }
    }

    fn impaired_link(impairment: crate::link::ManifestImpairment) -> Link2 {
        Link2 {
            src: "dev01::eth1".to_string(),
            dst: "dev02::eth1".to_string(),
            impairment: Some(impairment),
        }
    }

    #[test]
    fn manifest_links_013_an_impairment_netem_cannot_apply_is_refused_naming_the_link_and_field() {
        let percent = |field: &str, value: f64| {
            let mut impairment = crate::link::ManifestImpairment::default();
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
                crate::link::ManifestImpairment {
                    delay: Some(274_878),
                    ..Default::default()
                },
            ),
            (
                "jitter",
                crate::link::ManifestImpairment {
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
            let error = format!(
                "{:#}",
                check_link_impairments(&[impaired_link(impairment)]).unwrap_err()
            );
            assert!(error.contains("dev01::eth1 <-> dev02::eth1"), "{error}");
            assert!(error.contains(&format!("Impairment {field} ")), "{error}");
        }
        check_link_impairments(&[impaired_link(crate::link::ManifestImpairment {
            delay: Some(274_877),
            jitter: Some(274_877),
            loss_percent: Some(100.0),
            reorder_percent: Some(0.0),
            corrupt_percent: Some(1.5),
        })])
        .unwrap();
    }
}
