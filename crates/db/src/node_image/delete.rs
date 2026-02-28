use anyhow::{Context, Result, anyhow};
use shared::data::{NodeConfig, RecordId};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// Delete a node_image record from the database by its RecordId
///
/// Returns an error if:
/// - The record doesn't exist (returns "Node image not found" error)
/// - The image is referenced by any nodes (database constraint will reject deletion)
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
/// - If the record is referenced by nodes (REFERENCE ON DELETE REJECT constraint)
/// - If there's a database error during deletion
pub async fn delete_node_image(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<()> {
    // Execute DELETE query
    let deleted: Option<NodeConfig> = db
        .delete(id.clone())
        .await
        .context(format!(
            "Failed to delete node_image: {:?}\nNote: Deletion will fail if any nodes reference this image",
            id
        ))?;

    // Verify the record was found and deleted
    deleted.ok_or_else(|| anyhow!("Node image not found for deletion: {:?}", id))?;

    Ok(())
}
