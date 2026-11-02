use anyhow::{Context, Result, anyhow, ensure};
use shared::data::{NodeConfig, RecordId};
use tracing::instrument;

use crate::Database;
use crate::persistence::{NODE_IMAGE_TABLE, key_of, node_image_columns, node_image_from_row};
use crate::sql::{query_count, query_optional};

/// Delete a node_image record from the database by its RecordId
///
/// Returns an error if:
/// - The record doesn't exist (returns "Node image not found" error)
/// - The image is referenced by any nodes (`node.image` is `ON DELETE RESTRICT`)
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - RecordId of the node_image to delete
///
/// # Returns
/// `Ok(())` on successful deletion
///
/// # Errors
/// - If the record doesn't exist
/// - If the record is referenced by nodes
/// - If there's a database error during deletion
#[instrument(skip(db), level = "debug")]
pub async fn delete_node_image(db: &Database, id: RecordId) -> Result<()> {
    let key = key_of(&id, NODE_IMAGE_TABLE)?;
    let writer = db.writer().await?;

    // The foreign key refuses the delete as well; checking first gives a clearer message.
    // Both run under the write lock, so no node can start using the image in between.
    let references = query_count(
        &writer.conn,
        "SELECT count(*) FROM node WHERE image = ?1",
        [key.as_str()],
    )
    .await
    .context("Cannot check image references before deletion")?;
    ensure!(
        references == 0,
        "Cannot delete node_image {:?}: referenced by {} managed node(s)",
        id,
        references
    );

    let deleted: Option<NodeConfig> = query_optional(
        &writer.conn,
        concat!(
            "DELETE FROM node_image WHERE id = ?1 RETURNING ",
            node_image_columns!()
        ),
        [key.as_str()],
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to delete node_image: {:?}\nNote: Deletion will fail if any nodes reference this image",
        id
    ))?;

    // Verify the record was found and deleted
    deleted.ok_or_else(|| anyhow!("Node image not found for deletion: {:?}", id))?;

    Ok(())
}
