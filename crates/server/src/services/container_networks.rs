//! Docker network attachments for a container node's data interfaces.
//!
//! Docker names a container's interfaces in the order networks are attached, unless an
//! attachment names its interface. Every data interface must therefore get an attachment, in
//! interface order: one left out would rename every interface after it.

use anyhow::{Result, bail};

use shared::data;
use shared::util;

/// How a container data interface is connected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ContainerDataInterface {
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

/// Whether the Docker network `network` is one of `node_name`'s data networks in the lab
/// `lab_id`: `<node>-brs<n>-<lab_id>` or `<node>-iso<n>-<lab_id>`.
///
/// The whole name is checked, so redeploying node `r1` never takes the networks of a node
/// named `r1-x`.
pub(crate) fn is_node_network(network: &str, node_name: &str, lab_id: &str) -> bool {
    let Some(middle) = network
        .strip_prefix(node_name)
        .and_then(|rest| rest.strip_prefix('-'))
        .and_then(|rest| rest.strip_suffix(lab_id))
        .and_then(|rest| rest.strip_suffix('-'))
    else {
        return false;
    };
    ["brs", "iso"].iter().any(|kind| {
        middle
            .strip_prefix(kind)
            .is_some_and(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// The Docker attachment for one container data interface.
pub(crate) fn data_attachment(
    node_name: &str,
    model: data::NodeModel,
    interface_name: &str,
    interface_idx: u8,
    connection: ContainerDataInterface,
    lab_id: &str,
) -> Result<data::ContainerNetworkAttachment> {
    // SR Linux and cJunosEvolved need their own interface names, and IOL's bridge relays
    // `eth<N>` to port N. Other models are named by attach order, except disabled interfaces,
    // which are named so they stay down.
    let fixed_name = match model {
        data::NodeModel::NokiaSrlinux
        | data::NodeModel::JuniperCevolved
        | data::NodeModel::CiscoIol
        | data::NodeModel::CiscoIoll2 => {
            Some(util::container_interface_name(&model, interface_idx)?)
        }
        _ => None,
    };
    let (name, linux_interface_name, admin_down) = match connection {
        ContainerDataInterface::Bridged => (
            bridge_member_network_name(node_name, interface_idx, lab_id),
            fixed_name,
            false,
        ),
        ContainerDataInterface::Disabled => (
            isolated_network_name(node_name, interface_idx, lab_id),
            Some(fixed_name.unwrap_or_else(|| interface_name.to_string())),
            !model.keeps_data_interfaces_up(),
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

/// The Docker attachments for a container node's data interfaces, in interface order. Only a
/// container with no `[[links]]` link gets them: every link is a P2p link, and a container with
/// one takes all its data interfaces as veths instead.
pub(crate) fn data_attachments(
    node_name: &str,
    model: data::NodeModel,
    interfaces: &[data::InterfaceData],
    lab_id: &str,
) -> Result<Vec<data::ContainerNetworkAttachment>> {
    let mut attachments = vec![];
    for iface in interfaces {
        let connection = match &iface.data {
            data::NodeInterface::Peer(_) => {
                bail!(
                    "Link interface {} of node {} has no Docker network: a container with a link \
                     takes its data interfaces as veths",
                    iface.name,
                    node_name
                );
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
            }),
        )
    }

    #[test]
    fn a_nodes_networks_are_matched_by_whole_name() {
        assert!(is_node_network("r1-brs1-fe4edfdc", "r1", "fe4edfdc"));
        assert!(is_node_network("r1-iso12-fe4edfdc", "r1", "fe4edfdc"));
        // Node r1-x's networks, and other names that only share the ends.
        assert!(!is_node_network("r1-x-brs1-fe4edfdc", "r1", "fe4edfdc"));
        assert!(!is_node_network("r1-brs-fe4edfdc", "r1", "fe4edfdc"));
        assert!(!is_node_network("r1-brsx1-fe4edfdc", "r1", "fe4edfdc"));
        assert!(!is_node_network("r1-brs1-0a1b2c3d", "r1", "fe4edfdc"));
        assert!(!is_node_network(
            "sherpa-management-fe4edfdc",
            "r1",
            "fe4edfdc"
        ));
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
            data::NodeModel::FrrLinux,
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
            data::NodeModel::NokiaSrlinux,
            "eth-1/1",
            1,
            ContainerDataInterface::Bridged,
            LAB_ID,
        )
        .unwrap();
        assert_eq!(attachment.linux_interface_name.as_deref(), Some("e1-1"));
    }

    #[test]
    fn iol_interfaces_are_eth_by_index_bridged_or_disabled() {
        for model in [data::NodeModel::CiscoIol, data::NodeModel::CiscoIoll2] {
            let bridged = data_attachment(
                "r1",
                model,
                "Ethernet1/2",
                6,
                ContainerDataInterface::Bridged,
                LAB_ID,
            )
            .unwrap();
            assert_eq!(bridged.linux_interface_name.as_deref(), Some("eth6"));
            let disabled = data_attachment(
                "r1",
                model,
                "Ethernet0/3",
                3,
                ContainerDataInterface::Disabled,
                LAB_ID,
            )
            .unwrap();
            assert_eq!(disabled.linux_interface_name.as_deref(), Some("eth3"));
            assert!(disabled.admin_down);
        }
    }

    #[test]
    fn cevolved_interfaces_start_at_eth4() {
        let attachment = data_attachment(
            "cevo01",
            data::NodeModel::JuniperCevolved,
            "et-0/0/0",
            1,
            ContainerDataInterface::Bridged,
            LAB_ID,
        )
        .unwrap();
        assert_eq!(attachment.linux_interface_name.as_deref(), Some("eth4"));
        assert!(!attachment.admin_down);
    }

    #[test]
    fn cevolved_disabled_interfaces_stay_up() {
        // cJunosEvolved waits at boot until every data interface is up.
        let attachment = data_attachment(
            "cevo01",
            data::NodeModel::JuniperCevolved,
            "et-0/0/2",
            3,
            ContainerDataInterface::Disabled,
            LAB_ID,
        )
        .unwrap();
        assert_eq!(attachment.name, "cevo01-iso3-fe4edfdc");
        assert_eq!(attachment.linux_interface_name.as_deref(), Some("eth6"));
        assert!(!attachment.admin_down);
    }

    #[test]
    fn test_disabled_interface_is_named_and_admin_down() {
        let attachment = data_attachment(
            "fr01",
            data::NodeModel::FrrLinux,
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
    fn test_bridge_member_keeps_later_interfaces_in_order() {
        // A bridge member on eth1 used to be dropped, so later interfaces came up one name early.
        let interfaces = vec![
            interface("eth0", 0, data::NodeInterface::Management),
            bridged("eth1", 1),
            interface("eth2", 2, data::NodeInterface::Disabled),
            bridged("eth3", 3),
        ];
        let attachments =
            data_attachments("fr01", data::NodeModel::FrrLinux, &interfaces, LAB_ID).unwrap();
        let names: Vec<&str> = attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "fr01-brs1-fe4edfdc",
                "fr01-iso2-fe4edfdc",
                "fr01-brs3-fe4edfdc"
            ]
        );
    }

    #[test]
    fn test_link_interface_is_an_error() {
        let interfaces = vec![peer("eth2", 2)];
        let error = data_attachments("fr01", data::NodeModel::FrrLinux, &interfaces, LAB_ID)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("takes its data interfaces as veths"),
            "{error}"
        );
    }
}
