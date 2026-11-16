use anyhow::{Context, Result, anyhow, ensure};
use shared::data::{NodeConfig, RecordId};
use tracing::instrument;

use crate::Database;
use crate::persistence::{NODE_IMAGE_TABLE, enum_text, key_of, node_image_from_row};
use crate::queries;
use crate::sql::{query_count, query_optional};

/// Delete a `node_image` record by its `RecordId`, and keep its model with a default.
///
/// When the deleted image was its model's default, the most recently added remaining version of
/// the model becomes the default, in the same transaction. That version is returned; `None` when
/// the deleted image was not the default or was the model's last.
///
/// # Errors
/// - If the record doesn't exist
/// - If the record is referenced by nodes (`node.image` is `ON DELETE RESTRICT`)
/// - If there's a database error during deletion
#[instrument(skip(db), level = "debug")]
pub async fn delete_node_image(db: &Database, id: RecordId) -> Result<Option<NodeConfig>> {
    let key = key_of(&id, NODE_IMAGE_TABLE)?;
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the node_image delete transaction")?;

    // The foreign key refuses the delete as well; checking first gives a clearer message.
    // Both run under the write lock, so no node can start using the image in between.
    let references = query_count(
        &tx,
        queries::node::COUNT_BY_IMAGE.bind([key.as_str().into()]),
    )
    .await
    .context("Cannot check image references before deletion")?;
    ensure!(
        references == 0,
        "Cannot delete node_image {:?}: referenced by {} managed node(s)",
        id,
        references
    );

    let deleted = query_optional(
        &tx,
        queries::node_image::DELETE_BY_ID.bind([key.as_str().into()]),
        node_image_from_row,
    )
    .await
    .context(format!(
        "Failed to delete node_image: {id:?}\nNote: Deletion will fail if any nodes reference this image"
    ))?
    .ok_or_else(|| anyhow!("Node image not found for deletion: {id:?}"))?;

    let promoted = if deleted.default {
        query_optional(
            &tx,
            queries::node_image::PROMOTE_NEWEST.bind([
                enum_text(&deleted.model, "model")?.into(),
                enum_text(&deleted.kind, "kind")?.into(),
            ]),
            node_image_from_row,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to make another version the default for {} ({})",
                deleted.model, deleted.kind
            )
        })?
    } else {
        None
    };

    tx.commit()
        .await
        .context("Failed to commit the node_image delete transaction")?;
    Ok(promoted)
}
