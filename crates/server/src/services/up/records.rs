//! Phase 3: the lab's database records, its nodes' interfaces, and their isolated and reserved
//! networks.

use anyhow::{Context, Result, anyhow};
use tracing::instrument;

use super::context::LabBuild;
use super::links::{find_bridge_interface, find_interface_link};
use super::networks::ReservedLab;
use super::nodes::get_node_image;
use crate::services::node_ops;
use crate::services::node_resources;

use shared::data;
use shared::data::{LabSubnets, RecordId, StatusKind};
use shared::konst::SHERPA_MANAGEMENT_NETWORK_NAME;
use shared::util;
use shared::validators::effective_data_interface_count;

/// The lab's record and subnets, and its nodes: their records, kinds and interface plans.
pub(super) struct NodeRecords {
    pub(super) lab_record: data::DbLab,
    pub(super) lab_record_id: RecordId,
    pub(super) subnets: LabSubnets,
    pub(super) container_nodes: Vec<topology::NodeExpanded>,
    pub(super) unikernel_nodes: Vec<topology::NodeExpanded>,
    pub(super) vm_nodes: Vec<topology::NodeExpanded>,
    pub(super) lab_node_data: Vec<data::LabNodeData>,
    pub(super) node_setup_data: Vec<data::NodeSetupData>,
}

/// Record every node of the lab `up` reserved, and its resources, plan each node's interfaces,
/// and create the isolated and reserved networks of nodes that need them.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_node_records(
    build: &LabBuild<'_>,
    reserved: ReservedLab,
    nodes_expanded: &[topology::NodeExpanded],
    node_images: &[data::NodeConfig],
    links_detailed: &[topology::LinkDetailed],
    bridges_detailed: &[topology::BridgeDetailed],
) -> Result<NodeRecords> {
    let LabBuild { lab_id, db, .. } = *build;
    let ReservedLab {
        record: lab_record,
        subnets: networks,
    } = reserved;
    let management_network = format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}");

    tracing::info!(lab_id = %lab_id, lab_name = %lab_record.name, "Creating node database records");

    let lab_record_id = db::get_lab_id(&lab_record).context("Failed to get lab record ID")?;

    let mut container_nodes: Vec<topology::NodeExpanded> = vec![];
    let mut unikernel_nodes: Vec<topology::NodeExpanded> = vec![];
    let mut vm_nodes: Vec<topology::NodeExpanded> = vec![];

    let mut lab_node_data = vec![];
    let mut node_setup_data = vec![];

    for node in nodes_expanded {
        let node_image = get_node_image(node.model, node.version.as_deref(), node_images)?;

        tracing::info!(
            lab_id = %lab_id,
            node_name = %node.name,
            node_kind = ?node_image.kind,
            node_model = ?node_image.model,
            "Creating node database record"
        );

        let node_interfaces_detailed =
            plan_interfaces(node, &node_image, links_detailed, bridges_detailed)?;

        let lab_node = db::create_node(
            db,
            &node.name,
            node.index,
            db::get_image_id(&node_image)?,
            lab_record_id.clone(),
        )
        .await?;
        // Record its vCPUs, memory and, for a VM, its VNC console port.
        let lab_node = node_resources::reserve_node_resources(
            db,
            &lab_node,
            &node_image.kind,
            node_resources::planned_resources(node, &node_image),
        )
        .await?;

        lab_node_data.push(data::LabNodeData {
            name: node.name.clone(),
            model: node_image.model,
            kind: node_image.kind.clone(),
            index: node.index,
            record: lab_node,
        });

        match node_image.kind {
            data::NodeKind::Container => {
                container_nodes.push(node.clone());
            }
            data::NodeKind::Unikernel => {
                unikernel_nodes.push(node.clone());
            }
            data::NodeKind::VirtualMachine => {
                vm_nodes.push(node.clone());
            }
        }

        node_setup_data.push(
            node_setup(
                build,
                node,
                &node_image,
                node_interfaces_detailed,
                &management_network,
            )
            .await?,
        );
    }

    tracing::info!(
        lab_id = %lab_id,
        total_nodes = nodes_expanded.len(),
        containers = container_nodes.len(),
        vms = vm_nodes.len(),
        unikernels = unikernel_nodes.len(),
        "Created all node database records"
    );

    Ok(NodeRecords {
        lab_record,
        lab_record_id,
        subnets: networks,
        container_nodes,
        unikernel_nodes,
        vm_nodes,
        lab_node_data,
        node_setup_data,
    })
}

/// The node's interfaces, by index: management, reserved, then each data interface on its link
/// or bridge, or disabled.
fn plan_interfaces(
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    links_detailed: &[topology::LinkDetailed],
    bridges_detailed: &[topology::BridgeDetailed],
) -> Result<Vec<data::InterfaceData>> {
    let mut node_interfaces_detailed: Vec<data::InterfaceData> = vec![];
    let data_interface_count =
        effective_data_interface_count(&node.name, node.data_interface_count, node_image)?;
    let first_data_interface_idx = node_image
        .reserved_interface_count
        .checked_add(1)
        .ok_or_else(|| anyhow!("Reserved interface count overflow for node {}", node.name))?;
    let max_interface_idx = node_image
        .reserved_interface_count
        .checked_add(data_interface_count)
        .ok_or_else(|| anyhow!("Data interface count overflow for node {}", node.name))?;

    for idx in 0..=max_interface_idx {
        let interface_name = util::interface_from_idx(&node.model, idx)?;
        let interface_idx = idx;
        let mut interface_state = data::InterfaceState::Enabled;
        let mut interface_data = data::NodeInterface::Disabled;

        if idx == 0 {
            interface_data = data::NodeInterface::Management;
        } else if idx < first_data_interface_idx {
            interface_data = data::NodeInterface::Reserved;
        } else {
            if let Some(data) = find_interface_link(&node.name, &interface_name, links_detailed) {
                interface_data = data;
            }
            if let Some(data) = find_bridge_interface(&node.name, &interface_name, bridges_detailed)
            {
                interface_data = data;
            }
            interface_state = data::InterfaceState::Disabled;
        }

        node_interfaces_detailed.push(data::InterfaceData {
            name: interface_name.clone(),
            index: interface_idx,
            state: interface_state,
            data: interface_data,
        });
    }
    Ok(node_interfaces_detailed)
}

/// Create the node's isolated network, when it has disabled interfaces, and a VM's reserved
/// network, and return the node's setup data.
#[instrument(skip_all, fields(lab_id = %build.lab_id, node_name = %node.name), level = "debug")]
async fn node_setup(
    build: &LabBuild<'_>,
    node: &topology::NodeExpanded,
    node_image: &data::NodeConfig,
    node_interfaces_detailed: Vec<data::InterfaceData>,
    management_network: &str,
) -> Result<data::NodeSetupData> {
    let LabBuild {
        lab_id,
        qemu_conn,
        progress,
        ..
    } = *build;
    let has_disabled_interfaces = node_interfaces_detailed
        .iter()
        .any(|i| matches!(i.data, data::NodeInterface::Disabled));

    let node_isolated_network = if matches!(
        node_image.kind,
        data::NodeKind::VirtualMachine | data::NodeKind::Container
    ) && has_disabled_interfaces
    {
        Some(node_ops::node_isolated_network_data(
            &node.name, node.index, lab_id,
        ))
    } else {
        None
    };

    let node_reserved_network = if matches!(node_image.kind, data::NodeKind::VirtualMachine)
        && node_image.reserved_interface_count > 0
    {
        Some(node_ops::node_reserved_network_data(
            &node.name, node.index, lab_id,
        ))
    } else {
        None
    };

    if let Some(network) = node_isolated_network.clone() {
        let _ = progress.send_status(
            format!("Creating isolated network for node: {}", node.name),
            StatusKind::Progress,
        );
        tracing::info!(
            lab_id = %lab_id,
            node_name = %node.name,
            network_type = "isolated",
            "Creating node isolated network"
        );
        match node_image.kind {
            data::NodeKind::VirtualMachine => {
                let node_isolated_network = libvirt::IsolatedNetwork {
                    network_name: network.network_name,
                    bridge_name: network.bridge_name,
                };
                node_isolated_network.create(qemu_conn)?;
            }
            data::NodeKind::Container => {
                network::create_bridge(&network.bridge_name, &network.network_name).await?;
            }
            data::NodeKind::Unikernel => {
                tracing::warn!(
                    node_name = %node.name,
                    "Skipping isolated network creation for unikernel node"
                );
            }
        }
    }

    if let Some(network) = node_reserved_network.clone() {
        let _ = progress.send_status(
            format!("Creating reserved network for node: {}", node.name),
            StatusKind::Progress,
        );
        tracing::info!(
            lab_id = %lab_id,
            node_name = %node.name,
            network_type = "reserved",
            "Creating node reserved network"
        );
        let node_reserved_network = libvirt::ReservedNetwork {
            network_name: network.network_name,
            bridge_name: network.bridge_name,
        };
        node_reserved_network.create(qemu_conn)?;
    }

    Ok(data::NodeSetupData {
        name: node.name.clone(),
        index: node.index,
        management_network: management_network.to_owned(),
        isolated_network: node_isolated_network,
        reserved_network: node_reserved_network,
        interfaces: node_interfaces_detailed,
    })
}
