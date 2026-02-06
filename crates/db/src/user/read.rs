use anyhow::{Context, Result, anyhow};
use data::{DbUser, RecordId};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// Get a user by username
///
/// This is the primary way to retrieve users, as username is the unique identifier.
///
/// # Arguments
/// * `db` - Database connection
/// * `username` - The username to search for
///
/// # Returns
/// The DbUser if found
///
/// # Errors
/// - If the user is not found
/// - If there's a database error during the query
///
/// # Example
/// ```no_run
/// # use db::{connect, create_user, get_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// create_user(&db, "alice".to_string(), vec![]).await?;
///
/// let user = get_user(&db, "alice").await?;
/// assert_eq!(user.username, "alice");
/// # Ok(())
/// # }
/// ```
pub async fn get_user(db: &Surreal<Client>, username: &str) -> Result<DbUser> {
    let mut response = db
        .query("SELECT * FROM ONLY user WHERE username = $username")
        .bind(("username", username.to_string()))
        .await
        .context(format!("Failed to query user from database: {}", username))?;

    let user: Option<DbUser> = response.take(0)?;
    user.ok_or_else(|| anyhow!("User not found: {}", username))
}

/// Get a user by RecordId
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the user
///
/// # Returns
/// Some(DbUser) if found, None if not found
///
/// # Errors
/// - If there's a database error during the query
///
/// # Example
/// ```no_run
/// # use db::{connect, create_user, get_user_by_id};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let created = create_user(&db, "alice".to_string(), vec![]).await?;
/// let user_id = created.id.expect("User should have ID");
///
/// let user = get_user_by_id(&db, user_id).await?;
/// assert!(user.is_some());
/// # Ok(())
/// # }
/// ```
pub async fn get_user_by_id(db: &Surreal<Client>, id: RecordId) -> Result<Option<DbUser>> {
    let user: Option<DbUser> = db
        .select(id.clone())
        .await
        .context(format!("Failed to query user by id: {:?}", id))?;

    Ok(user)
}

/// List all users in the database
///
/// # Arguments
/// * `db` - Database connection
///
/// # Returns
/// A vector of all users
///
/// # Errors
/// - If there's a database error during the query
///
/// # Example
/// ```no_run
/// # use db::{connect, create_user, list_users};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// create_user(&db, "alice".to_string(), vec![]).await?;
/// create_user(&db, "bob".to_string(), vec![]).await?;
///
/// let users = list_users(&db).await?;
/// assert!(users.len() >= 2);
/// # Ok(())
/// # }
/// ```
pub async fn list_users(db: &Surreal<Client>) -> Result<Vec<DbUser>> {
    let users: Vec<DbUser> = db
        .select("user")
        .await
        .context("Failed to query all users from database")?;

    Ok(users)
}

/// Count the total number of users in the database
///
/// # Arguments
/// * `db` - Database connection
///
/// # Returns
/// The number of users
///
/// # Errors
/// - If there's a database error during the query
///
/// # Example
/// ```no_run
/// # use db::{connect, create_user, count_users};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
///
/// let initial_count = count_users(&db).await?;
/// create_user(&db, "alice".to_string(), vec![]).await?;
/// let new_count = count_users(&db).await?;
///
/// assert_eq!(new_count, initial_count + 1);
/// # Ok(())
/// # }
/// ```
pub async fn count_users(db: &Surreal<Client>) -> Result<usize> {
    let users: Vec<DbUser> = db
        .select("user")
        .await
        .context("Failed to count users from database")?;

    Ok(users.len())
}
