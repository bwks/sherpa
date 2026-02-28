use anyhow::{Context, Result, anyhow};
use shared::data::NodeConfig;
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::helpers::get_image_id;

/// Update an existing node_image record in the database
///
/// This performs a full replacement of all fields. The NodeConfig must have
/// a valid `id` field set.
///
/// # Arguments
/// * `db` - Database connection
/// * `config` - NodeConfig with updated fields and a valid `id`
///
/// # Returns
/// The updated NodeConfig on success
///
/// # Errors
/// - If the config has no ID
/// - If the record doesn't exist in the database
/// - If there's a database error during the update
pub async fn update_node_image(
    db: &Arc<Surreal<Client>>,
    config: NodeConfig,
) -> Result<NodeConfig> {
    // Extract and validate the ID
    let id = get_image_id(&config)?;

    // If setting default=true, unset default on other versions of same (model, kind)
    if config.default {
        db.query(
            "UPDATE node_image SET default = false
             WHERE model = $model AND kind = $kind AND id != $id",
        )
        .bind(("model", config.model.to_string()))
        .bind(("kind", config.kind.to_string()))
        .bind(("id", id.clone()))
        .await
        .context(format!(
            "Error unsetting default flag on other versions:\n model: {}\n kind: {}\n",
            config.model, config.kind
        ))?;
    }

    // Execute UPDATE query - replaces all fields
    let updated: Option<NodeConfig> = db
        .update(id.clone())
        .content(config.clone())
        .await
        .context(format!(
            "Failed to update node_image:\n id: {:?}\n model: {}\n kind: {}\n",
            id, config.model, config.kind
        ))?;

    // Return result or error if not found
    updated.ok_or_else(|| {
        anyhow!(
            "Node image not found for update:\n id: {:?}\n model: {}\n kind: {}\n",
            id,
            config.model,
            config.kind
        )
    })
}
