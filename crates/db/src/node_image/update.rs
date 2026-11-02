use anyhow::{Context, Result, anyhow};
use shared::data::NodeConfig;
use tracing::instrument;

use crate::Database;
use crate::helpers::get_image_id;
use crate::node_image::create::replace_node_image;
use crate::persistence::{NODE_IMAGE_TABLE, enum_text, key_of};

/// Update an existing node_image record in the database
///
/// This performs a full replacement of all fields. The NodeConfig must have
/// a valid `id` field set. If config.default is true, it also unsets default on the other
/// versions of the same (model, kind), in the same transaction.
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
#[instrument(skip(db), level = "debug")]
pub async fn update_node_image(db: &Database, config: NodeConfig) -> Result<NodeConfig> {
    // Extract and validate the ID
    let id = get_image_id(&config)?;
    let key = key_of(&id, NODE_IMAGE_TABLE)?;

    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the node_image update transaction")?;

    // If setting default=true, unset default on other versions of same (model, kind)
    if config.default {
        tx.execute(
            "UPDATE node_image SET \"default\" = 0 WHERE model = ?1 AND kind = ?2 AND id != ?3",
            (
                enum_text(&config.model, "model")?,
                enum_text(&config.kind, "kind")?,
                key.as_str(),
            ),
        )
        .await
        .context(format!(
            "Error unsetting default flag on other versions:\n model: {}\n kind: {}\n",
            config.model, config.kind
        ))?;
    }

    // Replace all fields
    let updated = replace_node_image(&tx, &config, &key)
        .await
        .context(format!(
            "Failed to update node_image:\n id: {:?}\n model: {}\n kind: {}\n",
            id, config.model, config.kind
        ))?;

    // Return result or error if not found; dropping the transaction rolls it back
    let updated = updated.ok_or_else(|| {
        anyhow!(
            "Node image not found for update:\n id: {:?}\n model: {}\n kind: {}\n",
            id,
            config.model,
            config.kind
        )
    })?;

    tx.commit()
        .await
        .context("Failed to commit the node_image update transaction")?;
    Ok(updated)
}
