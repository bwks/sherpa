use anyhow::{anyhow, Context, Result};
use data::DbLink;
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::link::read::get_link;

/// Update an existing link in the database
///
/// **IMPORTANT:** The `lab`, `node_a`, and `node_b` fields are immutable and cannot be changed.
/// If the provided link has different values for these fields than the existing link,
/// the update will fail. Links cannot be moved between labs or re-assigned to different nodes.
///
/// # Arguments
/// * `db` - Database connection
/// * `link` - DbLink with all fields populated (id field is required)
///
/// # Returns
/// The updated DbLink record
///
/// # Errors
/// - If link.id is None (id is required for updates)
/// - If link doesn't exist
/// - If trying to change the lab (lab field is immutable)
/// - If trying to change node_a or node_b (immutable after creation)
/// - If unique constraints are violated (node_a, node_b, int_a, int_b combination)
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, get_link, update_link};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # use surrealdb::RecordId;
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let id: RecordId = ("link", "abc123").into();
/// let mut link = get_link(&db, id).await?;
/// link.int_a = "eth1".to_string();
/// link.int_b = "eth1".to_string();
/// let updated = update_link(&db, link).await?;
/// assert_eq!(updated.int_a, "eth1");
/// # Ok(())
/// # }
/// ```
pub async fn update_link(db: &Surreal<Client>, link: DbLink) -> Result<DbLink> {
    // Require id field for updates
    let id = link
        .id
        .as_ref()
        .ok_or_else(|| anyhow!("Cannot update link without id field"))?;

    // Verify the link exists and check if immutable fields are being changed
    let existing = get_link(db, id.clone()).await?;

    // Verify lab is not being changed - it's immutable
    if existing.lab != link.lab {
        return Err(anyhow!(
            "Cannot change link lab: lab field is immutable. Links cannot be moved between labs. Existing lab: {}, attempted new lab: {}",
            existing.lab,
            link.lab
        ));
    }

    // Verify node_a is not being changed - it's immutable
    if existing.node_a != link.node_a {
        return Err(anyhow!(
            "Cannot change link node_a: node_a field is immutable. Existing node_a: {}, attempted new node_a: {}",
            existing.node_a,
            link.node_a
        ));
    }

    // Verify node_b is not being changed - it's immutable
    if existing.node_b != link.node_b {
        return Err(anyhow!(
            "Cannot change link node_b: node_b field is immutable. Existing node_b: {}, attempted new node_b: {}",
            existing.node_b,
            link.node_b
        ));
    }

    // Perform update
    let updated: Option<DbLink> = db
        .update(id.clone())
        .content(link.clone())
        .await
        .context(format!(
            "Failed to update link: node_a={}, node_b={}",
            link.node_a, link.node_b
        ))?;

    updated.ok_or_else(|| {
        anyhow!(
            "Link update failed: node_a={}, node_b={}",
            link.node_a,
            link.node_b
        )
    })
}
