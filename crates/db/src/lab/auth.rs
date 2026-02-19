use anyhow::{Context, Result, anyhow};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use surrealdb_types::SurrealValue;

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
pub async fn get_lab_owner_username(db: &Arc<Surreal<Client>>, lab_id: &str) -> Result<String> {
    let mut response = db
        .query("SELECT user.username AS username FROM ONLY lab WHERE lab_id = $lab_id")
        .bind(("lab_id", lab_id.to_string()))
        .await
        .context(format!(
            "Failed to query lab owner from database: {}",
            lab_id
        ))?;

    #[derive(serde::Deserialize, SurrealValue)]
    struct OwnerResult {
        username: String,
    }

    let result: Option<OwnerResult> = response.take(0)?;
    result
        .map(|r| r.username)
        .ok_or_else(|| anyhow!("Lab with lab_id not found or owner not found: {}", lab_id))
}
