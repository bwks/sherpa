use anyhow::{Context, Result, anyhow};
use tracing::instrument;
use turso::Row;

use crate::Database;
use crate::persistence::Cells;
use crate::sql::query_optional;

/// Get the username associated with a lab
///
/// This function fetches the lab and extracts the username from the owner's user record.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Returns
/// The username of the lab owner
///
/// # Errors
/// - If lab with lab_id not found
/// - If the user record cannot be found
/// - If there's a database error
#[instrument(skip(db), level = "debug")]
pub async fn get_lab_owner_username(db: &Database, lab_id: &str) -> Result<String> {
    let conn = db.reader().await?;
    let username = query_optional(
        &conn,
        "SELECT user.username FROM lab JOIN user ON user.id = lab.user WHERE lab.lab_id = ?1",
        [lab_id],
        username_from_row,
    )
    .await
    .context(format!(
        "Failed to query lab owner from database: {}",
        lab_id
    ))?;

    username.ok_or_else(|| anyhow!("Lab with lab_id not found or owner not found: {}", lab_id))
}

fn username_from_row(row: &Row) -> Result<String> {
    Cells::new(row).text("username")
}
