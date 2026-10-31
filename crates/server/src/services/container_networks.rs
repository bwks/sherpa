//! Docker network attachments for a container node's data interfaces.
//!
//! Docker names a container's interfaces in the order networks are attached, unless an
//! attachment names its interface. Every data interface must therefore get an attachment, in
//! interface order: one left out would rename every interface after it.

use std::collections::HashMap;

use anyhow::{Result, anyhow};

use shared::data;
use shared::util;

/// How a container data interface is connected.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ContainerDataInterface {
    /// One end of a `[[links]]` link, on the named Docker network.
    Linked(String),
    /// A member of a `[[bridges]]` private bridge.
    Bridged,
    /// Not connected: on the node's isolated network, admin down.
    Disabled,
}

/// Docker network of a container interface in a private bridge. Macvlan, bridge mode, on the
/// bridge's `brs` Linux bridge.
pub(crate) fn bridge_member_network_name(
    node_name: &str,
    interface_idx: u8,
    lab_id: &str,
) -> String {
    format!("{node_name}-brs{interface_idx}-{lab_id}")
}

/// Docker network of a disabled container interface, on the node's isolated bridge.
pub(crate) fn isolated_network_name(node_name: &str, interface_idx: u8, lab_id: &str) -> String {
    format!("{node_name}-iso{interface_idx}-{lab_id}")
}

/// The Docker attachment for one container data interface.
pub(crate) fn data_attachment(
    node_name: &str,
    model: &data::NodeModel,
    interface_name: &str,
    interface_idx: u8,
    connection: ContainerDataInterface,
    lab_id: &str,
) -> Result<data::ContainerNetworkAttachment> {
    // SR Linux needs its own interface names. Other models are named by attach order, except
    // disabled interfaces, which are named so they stay down.
    let srlinux_name = if *model == data::NodeModel::NokiaSrlinux {
        Some(util::srlinux_to_linux_interface(interface_name)?)
    } else {
        None
    };
    let (name, linux_interface_name, admin_down) = match connection {
        ContainerDataInterface::Linked(network) => (network, srlinux_name, false),
        ContainerDataInterface::Bridged => (
            bridge_member_network_name(node_name, interface_idx, lab_id),
            srlinux_name,
            false,
        ),
        ContainerDataInterface::Disabled => (
            isolated_network_name(node_name, interface_idx, lab_id),
            Some(srlinux_name.unwrap_or_else(|| interface_name.to_string())),
            true,
        ),
    };
    Ok(data::ContainerNetworkAttachment {
        name,
        ipv4_address: None,
        ipv6_address: None,
        linux_interface_name,
        admin_down,
    })
}

/// The Docker attachments for a container node's data interfaces, in interface order.
/// `link_networks` maps `(node name, interface name)` to the Docker network of a link end.
pub(crate) fn data_attachments(
    node_name: &str,
    model: &data::NodeModel,
    interfaces: &[data::InterfaceData],
    link_networks: &HashMap<(String, String), String>,
    lab_id: &str,
) -> Result<Vec<data::ContainerNetworkAttachment>> {
    let mut attachments = vec![];
    for iface in interfaces {
        let connection = match &iface.data {
            data::NodeInterface::Peer(_) => {
                let network = link_networks
                    .get(&(node_name.to_string(), iface.name.clone()))
                    .ok_or_else(|| {
                        anyhow!(
                            "No Docker network for link interface {} of node {}",
                            iface.name,
                            node_name
                        )
                    })?;
                ContainerDataInterface::Linked(network.clone())
            }
            data::NodeInterface::Bridge(_) => ContainerDataInterface::Bridged,
            data::NodeInterface::Disabled => ContainerDataInterface::Disabled,
            data::NodeInterface::Management | data::NodeInterface::Reserved => {
                tracing::debug!(
                    node = %node_name,
                    interface = %iface.name,
                    "Skipping management or reserved interface: not a data network"
                );
                continue;
            }
        };
        attachments.push(data_attachment(
            node_name,
            model,
            &iface.name,
            iface.index,
            connection,
            lab_id,
        )?);
    }
    Ok(attachments)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAB_ID: &str = "fe4edfdc";

    fn interface(name: &str, index: u8, data: data::NodeInterface) -> data::InterfaceData {
        data::InterfaceData {
            name: name.to_string(),
            index,
            state: data::InterfaceState::Enabled,
            data,
        }
    }

    fn bridged(name: &str, index: u8) -> data::InterfaceData {
        interface(
            name,
            index,
            data::NodeInterface::Bridge(data::BridgeInterface {
                name: format!("brs0-{LAB_ID}"),
            }),
        )
    }

    fn peer(name: &str, index: u8) -> data::InterfaceData {
        interface(
            name,
            index,
            data::NodeInterface::Peer(data::PeerInterface {
                link_index: 0,
                this_node: "fr01".to_string(),
                this_node_index: 2,
                this_interface: name.to_string(),
                this_interface_index: index,
                this_side: data::PeerSide::A,
                peer_node: "fr02".to_string(),
                peer_node_index: 3,
                peer_interface: name.to_string(),
                peer_interface_index: index,
                peer_side: data::PeerSide::B,
                p2p: false,
            }),
        )
    }

    fn link_networks() -> HashMap<(String, String), String> {
        HashMap::from([(
            ("fr01".to_string(), "eth2".to_string()),
            format!("fr01-etha0-{LAB_ID}"),
        )])
    }

    #[test]
    fn test_network_names_carry_node_interface_and_lab() {
        assert_eq!(
            bridge_member_network_name("fr01", 1, LAB_ID),
            "fr01-brs1-fe4edfdc"
        );
        assert_eq!(
            isolated_network_name("fr01", 3, LAB_ID),
            "fr01-iso3-fe4edfdc"
        );
    }

    #[test]
    fn test_bridge_member_gets_an_active_attachment() {
        let attachment = data_attachment(
            "fr01",
            &data::NodeModel::FrrLinux,
            "eth1",
            1,
            ContainerDataInterface::Bridged,
            LAB_ID,
        )
        .unwrap();
        assert_eq!(attachment.name, "fr01-brs1-fe4edfdc");
        assert!(!attachment.admin_down);
        assert_eq!(attachment.linux_interface_name, None);
    }

    #[test]
    fn test_srlinux_bridge_member_is_named() {
        let attachment = data_attachment(
            "srl01",
            &data::NodeModel::NokiaSrlinux,
            "eth-1/1",
            1,
            ContainerDataInterface::Bridged,
            LAB_ID,
        )
        .unwrap();
        assert_eq!(attachment.linux_interface_name.as_deref(), Some("e1-1"));
    }

    #[test]
    fn test_disabled_interface_is_named_and_admin_down() {
        let attachment = data_attachment(
            "fr01",
            &data::NodeModel::FrrLinux,
            "eth3",
            3,
            ContainerDataInterface::Disabled,
            LAB_ID,
        )
        .unwrap();
        assert_eq!(attachment.name, "fr01-iso3-fe4edfdc");
        assert!(attachment.admin_down);
        assert_eq!(attachment.linux_interface_name.as_deref(), Some("eth3"));
    }

    #[test]
    fn test_linked_interface_uses_its_link_network() {
        let attachment = data_attachment(
            "fr01",
            &data::NodeModel::FrrLinux,
            "eth2",
            2,
            ContainerDataInterface::Linked(format!("fr01-etha0-{LAB_ID}")),
            LAB_ID,
        )
        .unwrap();
        assert_eq!(attachment.name, "fr01-etha0-fe4edfdc");
        assert!(!attachment.admin_down);
    }

    #[test]
    fn test_bridge_member_keeps_later_interfaces_in_order() {
        // A bridge member on eth1 used to be dropped, so the eth2 link came up as eth1.
        let interfaces = vec![
            interface("eth0", 0, data::NodeInterface::Management),
            bridged("eth1", 1),
            peer("eth2", 2),
            interface("eth3", 3, data::NodeInterface::Disabled),
        ];
        let attachments = data_attachments(
            "fr01",
            &data::NodeModel::FrrLinux,
            &interfaces,
            &link_networks(),
            LAB_ID,
        )
        .unwrap();
        let names: Vec<&str> = attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "fr01-brs1-fe4edfdc",
                "fr01-etha0-fe4edfdc",
                "fr01-iso3-fe4edfdc"
            ]
        );
    }

    #[test]
    fn test_link_without_a_network_is_an_error() {
        let interfaces = vec![peer("eth2", 2)];
        let result = data_attachments(
            "fr01",
            &data::NodeModel::FrrLinux,
            &interfaces,
            &HashMap::new(),
            LAB_ID,
        );
        assert!(result.is_err());
    }
}
