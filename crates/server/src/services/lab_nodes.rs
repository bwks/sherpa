//! The nodes a lab action works on: the whole lab, or one node of it.

use anyhow::{Context, Result, anyhow};
use tracing::instrument;

use shared::data::{DbNode, NodeConfig, RecordId};

/// The lab's nodes, or only `node_name` when one is given, and their images. A named node that
/// is not in the lab is an error.
#[instrument(skip(db), fields(%lab_id), level = "debug")]
pub(crate) async fn load_target_nodes(
    db: &db::Database,
    lab_record_id: RecordId,
    lab_id: &str,
    node_name: Option<&str>,
) -> Result<(Vec<DbNode>, Vec<NodeConfig>)> {
    let db_nodes = db::list_nodes_by_lab(db, lab_record_id)
        .await
        .context("Failed to list nodes for lab")?;

    let target_nodes: Vec<_> = if let Some(name) = node_name {
        db_nodes.into_iter().filter(|n| n.name == name).collect()
    } else {
        db_nodes
    };

    if let Some(name) = node_name
        && target_nodes.is_empty()
    {
        return Err(anyhow!("Node '{name}' not found in lab '{lab_id}'"));
    }

    // Batch-fetch node images to determine kind (VM vs Container)
    let mut image_ids: Vec<RecordId> = target_nodes.iter().map(|n| n.image.clone()).collect();
    image_ids.dedup();

    let node_images = db::list_node_images_by_ids(db, image_ids)
        .await
        .context("Failed to batch fetch node images")?;
    Ok((target_nodes, node_images))
}
