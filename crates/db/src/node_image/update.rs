use anyhow::{Context, Result, anyhow};
use shared::data::NodeConfig;
use shared::validators::check_ztp;
use tracing::instrument;

use crate::Database;
use crate::helpers::get_image_id;
use crate::node_image::create::replace_node_image;
use crate::node_image::default::{DefaultRequest, claim_default};
use crate::persistence::{NODE_IMAGE_TABLE, key_of, node_image_from_row};
use crate::queries;
use crate::sql::query_optional;

/// Update the settings of an existing `node_image` record.
///
/// Every setting is replaced with `config`'s, except the record's identity: its model, kind and
/// version are kept as stored, since they name the image's files. The `default` flag is decided
/// by `request` (see [`DefaultRequest`]); `config.default` is ignored. It runs in one transaction
/// under the write lock.
///
/// # Errors
///
/// - If the config has no ID, or no record has it.
/// - If the config's ZTP setting is not one the record's model takes.
/// - With [`LastDefaultImage`](crate::LastDefaultImage) when `request` is
///   [`DefaultRequest::Unset`] on the model's only default.
/// - If there's a database error during the update.
#[instrument(skip(db), level = "debug")]
pub async fn update_node_image(
    db: &Database,
    config: NodeConfig,
    request: DefaultRequest,
) -> Result<NodeConfig> {
    let id = get_image_id(&config)?;
    let key = key_of(&id, NODE_IMAGE_TABLE)?;

    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the node_image update transaction")?;

    let stored = query_optional(
        &tx,
        queries::node_image::FIND_BY_ID.bind([key.as_str().into()]),
        node_image_from_row,
    )
    .await
    .context(format!("Failed to read node_image {id:?} for update"))?
    .ok_or_else(|| anyhow!("Node image not found for update: {id:?}"))?;

    check_ztp(stored.model, config.ztp.method)
        .context(format!("Refusing to update node_image {id:?}"))?;
    let default = claim_default(&tx, stored.model, &stored.kind, &stored.version, request).await?;
    let config = NodeConfig {
        model: stored.model,
        kind: stored.kind,
        version: stored.version,
        default,
        ..config
    };

    let updated = replace_node_image(&tx, &config, &key)
        .await?
        .ok_or_else(|| anyhow!("Node image not found for update: {id:?}"))?;

    tx.commit()
        .await
        .context("Failed to commit the node_image update transaction")?;
    Ok(updated)
}
