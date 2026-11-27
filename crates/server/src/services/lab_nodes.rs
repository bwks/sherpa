//! The nodes a lab action works on: the whole lab, or one node of it.

use std::collections::HashSet;

use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::{DbNode, ModelKind, NodeActionResult, NodeConfig, RecordId};

use crate::services::error::{SandboxRefusal, ServiceError};

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
        return Err(
            ServiceError::NotFound(format!("Node '{name}' not found in lab '{lab_id}'")).into(),
        );
    }

    // Batch-fetch node images to determine kind (VM vs Container)
    let mut image_ids: Vec<RecordId> = target_nodes.iter().map(|n| n.image.clone()).collect();
    image_ids.dedup();

    let node_images = db::list_node_images_by_ids(db, image_ids)
        .await
        .context("Failed to batch fetch node images")?;
    Ok((target_nodes, node_images))
}

/// `nodes` without those whose model runs in a sandbox but that lab `lab_record_id` has no
/// sandbox recorded for, as they were made before their model ran in one, and a refusal of each.
#[instrument(skip_all, level = "debug")]
pub(crate) async fn refuse_nodes_without_sandbox(
    db: &db::Database,
    lab_record_id: &RecordId,
    nodes: Vec<DbNode>,
    node_images: &[NodeConfig],
) -> Result<(Vec<DbNode>, Vec<NodeActionResult>)> {
    let sandboxes: HashSet<RecordId> = db::list_sandbox_nodes(db, lab_record_id)
        .await?
        .into_iter()
        .map(|sandbox| sandbox.node)
        .collect();
    let (refused, kept): (Vec<DbNode>, Vec<DbNode>) = nodes.into_iter().partition(|node| {
        node_images
            .iter()
            .find(|image| image.id.as_ref() == Some(&node.image))
            .and_then(|image| image.model_kind().ok())
            .is_some_and(ModelKind::runs_in_sandbox)
            && node.id.as_ref().is_none_or(|id| !sandboxes.contains(id))
    });
    let refusals = refused
        .into_iter()
        .map(|node| NodeActionResult {
            message: SandboxRefusal::AddressMissing {
                node: node.name.clone(),
            }
            .to_string(),
            name: node.name,
            success: false,
            links: Vec::new(),
        })
        .collect();
    Ok((kept, refusals))
}
