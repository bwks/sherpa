use anyhow::{Context, Result, anyhow, bail};
use shared::data::{DbLab, DbUser, RecordId};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::helpers::get_user_id;

/// Delete a user by RecordId
///
/// **WARNING:** This will CASCADE delete all labs owned by this user, along with
/// all nodes and links in those labs. Use `delete_user_safe()` if you want to
/// prevent deletion when the user owns labs.
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - RecordId of the user to delete
///
/// # Returns
/// `Ok(())` on successful deletion
///
/// # Errors
/// - If the record doesn't exist
/// - If there's a database error during deletion
///
/// # Example
/// ```no_run
/// # use db::{connect, create_user, delete_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
///
/// // Create a user
/// let user = create_user(&db, "alice".to_string(), vec![]).await?;
/// let user_id = user.id.expect("User should have ID");
///
/// // Delete it (will cascade delete all labs owned by this user)
/// delete_user(&db, user_id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_user(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<()> {
    // Execute DELETE query
    let deleted: Option<DbUser> = db.delete(id.clone()).await.context(format!(
        "Failed to delete user: {:?}\nNote: This will cascade delete all labs owned by this user",
        id
    ))?;

    // Verify the record was found and deleted
    deleted.ok_or_else(|| anyhow!("User not found for deletion: {:?}", id))?;

    Ok(())
}

/// Delete a user by username (convenience function)
///
/// **WARNING:** This will CASCADE delete all labs owned by this user, along with
/// all nodes and links in those labs. Use `delete_user_safe()` if you want to
/// prevent deletion when the user owns labs.
///
/// # Arguments
/// * `db` - Database connection
/// * `username` - Username of the user to delete
///
/// # Returns
/// `Ok(())` on successful deletion
///
/// # Errors
/// - If the user is not found
/// - If there's a database error during deletion
///
/// # Example
/// ```no_run
/// # use db::{connect, create_user, delete_user_by_username};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
///
/// create_user(&db, "alice".to_string(), vec![]).await?;
///
/// // Delete by username
/// delete_user_by_username(&db, "alice").await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_user_by_username(db: &Arc<Surreal<Client>>, username: &str) -> Result<()> {
    // First get the user to obtain their ID
    let mut response = db
        .query("SELECT * FROM ONLY user WHERE username = $username")
        .bind(("username", username.to_string()))
        .await
        .context(format!("Failed to query user: {}", username))?;

    let user: Option<DbUser> = response.take(0)?;
    let user = user.ok_or_else(|| anyhow!("User not found: {}", username))?;

    let id = get_user_id(&user)?;

    // Delete using the ID
    delete_user(db, id).await
}

/// Safely delete a user (fails if user owns any labs)
///
/// This function checks if the user owns any labs before attempting deletion.
/// If labs exist, it returns an error with information about the labs.
/// This prevents accidental data loss from cascade deletion.
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - RecordId of the user to delete
///
/// # Returns
/// `Ok(())` on successful deletion (only if user has no labs)
///
/// # Errors
/// - If the user owns labs (provides count and prevents deletion)
/// - If the record doesn't exist
/// - If there's a database error during the operation
///
/// # Example
/// ```no_run
/// # use db::{connect, create_user, create_lab, delete_user_safe};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
///
/// let user = create_user(&db, "alice".to_string(), vec![]).await?;
/// let user_id = user.id.clone().expect("User should have ID");
///
/// // Create a lab for this user
/// create_lab(&db, "test-lab", "lab-001", &user).await?;
///
/// // This will fail because user owns a lab
/// let result = delete_user_safe(&db, user_id).await;
/// assert!(result.is_err());
/// assert!(result.unwrap_err().to_string().contains("owns 1 lab"));
/// # Ok(())
/// # }
/// ```
pub async fn delete_user_safe(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<()> {
    // First check if the user exists
    let user: Option<DbUser> = db
        .select(id.clone())
        .await
        .context(format!("Failed to query user by id: {:?}", id))?;

    let _user = user.ok_or_else(|| anyhow!("User not found for deletion: {:?}", id))?;

    // Check if user owns any labs
    let mut response = db
        .query("SELECT * FROM lab WHERE user = $user_id")
        .bind(("user_id", id.clone()))
        .await
        .context(format!("Failed to check labs for user: {:?}", id))?;

    let labs: Vec<DbLab> = response.take(0)?;

    if !labs.is_empty() {
        bail!(
            "Cannot delete user: user owns {} lab(s). Delete the labs first or use delete_user() to force cascade deletion.\nUser ID: {:?}\nLabs: {:?}",
            labs.len(),
            id,
            labs.iter().map(|l| &l.name).collect::<Vec<_>>()
        );
    }

    // No labs found, safe to delete
    delete_user(db, id).await
}
