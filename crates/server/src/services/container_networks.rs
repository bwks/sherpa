//! Docker network attachments for the data interfaces of a container on no link and no private
//! bridge: each is disabled, on the node's isolated network. A container on a link or a bridge
//! takes its data interfaces as veths instead.
//!
//! Docker names a container's interfaces in the order networks are attached, unless an
//! attachment names its interface. Every data interface must therefore get an attachment, in
//! interface order: one left out would rename every interface after it.

use anyhow::{Result, bail};

use shared::data;
use shared::util;

/// Docker network of a disabled container interface, on the node's isolated bridge.
pub(crate) fn isolated_network_name(node_name: &str, interface_idx: u8, lab_id: &str) -> String {
    format!("{node_name}-iso{interface_idx}-{lab_id}")
}

/// Whether the Docker network `network` is one of `node_name`'s data networks in the lab
/// `lab_id`: `<node>-iso<n>-<lab_id>`.
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
    middle
        .strip_prefix("iso")
        .is_some_and(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()))
}

/// The Docker attachment for one disabled container data interface: named, so it stays down.
pub(crate) fn data_attachment(
    node_name: &str,
    model_kind: data::ModelKind,
    interface_name: &str,
    interface_idx: u8,
    lab_id: &str,
) -> Result<data::ContainerNetworkAttachment> {
    // SR Linux and cJunosEvolved need their own interface names, and IOL's bridge relays
    // `eth<N>` to port N.
    let linux_interface_name = match model_kind.model() {
        data::NodeModel::NokiaSrlinux
        | data::NodeModel::JuniperEvolved
        | data::NodeModel::CiscoIol
        | data::NodeModel::CiscoIoll2 => util::container_interface_name(model_kind, interface_idx)?,
        _ => interface_name.to_string(),
    };
    Ok(data::ContainerNetworkAttachment {
        name: isolated_network_name(node_name, interface_idx, lab_id),
        ipv4_address: None,
        ipv6_address: None,
        linux_interface_name: Some(linux_interface_name),
        admin_down: !model_kind.keeps_data_interfaces_up(),
    })
}

/// The Docker attachments for the data interfaces of a container on no link and no private
/// bridge, in interface order.
pub(crate) fn data_attachments(
    node_name: &str,
    model_kind: data::ModelKind,
    interfaces: &[data::InterfaceData],
    lab_id: &str,
) -> Result<Vec<data::ContainerNetworkAttachment>> {
    let mut attachments = vec![];
    for iface in interfaces {
        match &iface.data {
            data::NodeInterface::Peer(_) | data::NodeInterface::Bridge(_) => {
                bail!(
                    "Interface {} of node {} has no Docker network: a container on a link or a \
                     bridge takes its data interfaces as veths",
                    iface.name,
                    node_name
                );
            }
            data::NodeInterface::Disabled => {}
            data::NodeInterface::Management | data::NodeInterface::Reserved => {
                tracing::debug!(
                    node = %node_name,
                    interface = %iface.name,
                    "Skipping management or reserved interface: not a data network"
                );
                continue;
            }
        }
        attachments.push(data_attachment(
            node_name,
            model_kind,
            &iface.name,
            iface.index,
            lab_id,
        )?);
    }
    Ok(attachments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::ModelKind;

    const LAB_ID: &str = "fe4edfdc";

    fn interface(name: &str, index: u8, data: data::NodeInterface) -> data::InterfaceData {
        data::InterfaceData {
            name: name.to_string(),
            index,
            state: data::InterfaceState::Enabled,
            data,
        }
    }

    #[test]
    fn link_cleanup_004_a_redeploy_takes_only_its_own_nodes_docker_networks() {
        assert!(is_node_network("r1-iso1-fe4edfdc", "r1", "fe4edfdc"));
        assert!(is_node_network("r1-iso12-fe4edfdc", "r1", "fe4edfdc"));
        assert!(!is_node_network("r1-x-iso1-fe4edfdc", "r1", "fe4edfdc"));
        assert!(!is_node_network("r1-iso-fe4edfdc", "r1", "fe4edfdc"));
        assert!(!is_node_network("r1-isox1-fe4edfdc", "r1", "fe4edfdc"));
        assert!(!is_node_network("r1-iso1-0a1b2c3d", "r1", "fe4edfdc"));
        assert!(!is_node_network(
            "sherpa-management-fe4edfdc",
            "r1",
            "fe4edfdc"
        ));
    }

    #[test]
    fn node_interfaces_007_an_unused_interface_of_a_container_on_no_link_or_bridge_is_named_and_down()
     {
        let attachment = data_attachment(
            "fr01",
            ModelKind::default_for(data::NodeModel::FrrLinux),
            "eth3",
            3,
            LAB_ID,
        )
        .unwrap();
        assert_eq!(attachment.name, "fr01-iso3-fe4edfdc");
        assert!(attachment.admin_down);
        assert_eq!(attachment.linux_interface_name.as_deref(), Some("eth3"));

        for model in [data::NodeModel::CiscoIol, data::NodeModel::CiscoIoll2] {
            let disabled = data_attachment(
                "r1",
                ModelKind::default_for(model),
                "Ethernet0/3",
                3,
                LAB_ID,
            )
            .unwrap();
            assert_eq!(disabled.linux_interface_name.as_deref(), Some("eth3"));
            assert!(disabled.admin_down);
        }

        let attachment = data_attachment(
            "cevo01",
            ModelKind::new(
                data::NodeModel::JuniperEvolved,
                Some(data::NodeKind::Container),
            )
            .unwrap(),
            "et-0/0/2",
            3,
            LAB_ID,
        )
        .unwrap();
        assert_eq!(attachment.name, "cevo01-iso3-fe4edfdc");
        assert_eq!(attachment.linux_interface_name.as_deref(), Some("eth6"));
        assert!(!attachment.admin_down);

        let interfaces = vec![
            interface("eth0", 0, data::NodeInterface::Management),
            interface("eth1", 1, data::NodeInterface::Disabled),
            interface("eth2", 2, data::NodeInterface::Disabled),
        ];
        let attachments = data_attachments(
            "fr01",
            ModelKind::default_for(data::NodeModel::FrrLinux),
            &interfaces,
            LAB_ID,
        )
        .unwrap();
        let names: Vec<&str> = attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["fr01-iso1-fe4edfdc", "fr01-iso2-fe4edfdc"]);
    }
}
