//! Stages 2 and 3: destroying the node, and recreating a VM's own networks.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use tracing::instrument;

use super::context::Redeploy;
use crate::services::node_ops;

use shared::data;
use shared::data::StatusKind;

/// Destroy the node and its configuration directory (stage 2).
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
pub(super) async fn destroy_node(redeploy: &Redeploy<'_>) -> Result<()> {
    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let _ = redeploy.progress.send_status(
        format!("Destroying existing node: {}", node_name),
        StatusKind::Progress,
    );

    match redeploy.node_image.kind {
        data::NodeKind::VirtualMachine => {
            node_ops::destroy_vm_node(
                redeploy.qemu_conn.clone(),
                node_name,
                lab_id,
                redeploy.db_node.index,
                redeploy.node_image.reserved_interface_count,
            )
            .await?;
        }
        data::NodeKind::Container => {
            node_ops::destroy_container_node(&redeploy.state.docker, node_name, lab_id).await?;
        }
        data::NodeKind::Unikernel => {
            bail!("Redeploy not supported for unikernel nodes");
        }
    }

    let node_ztp_dir = format!("{}/{node_name}", redeploy.lab_dir);
    if Path::new(&node_ztp_dir).exists() {
        fs::remove_dir_all(&node_ztp_dir)
            .with_context(|| format!("Failed to remove ZTP dir: {}", node_ztp_dir))?;
    }

    let _ = redeploy.progress.send_status(
        format!("Existing node destroyed: {}", node_name),
        StatusKind::Done,
    );
    Ok(())
}

/// Recreate a VM's isolated network, and its reserved network if it has one (stage 3).
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
pub(super) fn recreate_vm_networks(redeploy: &Redeploy<'_>) -> Result<()> {
    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let node_idx = redeploy.db_node.index;
    let _ = redeploy.progress.send_status(
        format!("Recreating networks for node: {}", node_name),
        StatusKind::Progress,
    );

    let isolated = node_ops::node_isolated_network_data(node_name, node_idx, lab_id);
    libvirt::IsolatedNetwork {
        network_name: isolated.network_name,
        bridge_name: isolated.bridge_name,
    }
    .create(&redeploy.qemu_conn)?;

    if redeploy.node_image.reserved_interface_count > 0 {
        let reserved = node_ops::node_reserved_network_data(node_name, node_idx, lab_id);
        libvirt::ReservedNetwork {
            network_name: reserved.network_name,
            bridge_name: reserved.bridge_name,
        }
        .create(&redeploy.qemu_conn)?;
    }

    let _ = redeploy.progress.send_status(
        format!("Networks recreated for node: {}", node_name),
        StatusKind::Done,
    );
    Ok(())
}
