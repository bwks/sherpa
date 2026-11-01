use anyhow::{Context, Result, anyhow};
use shared::data::{DbUser, RecordId};
use shared::util::normalize_username;
use tracing::instrument;

use crate::Database;
use crate::persistence::{USER_TABLE, key_of, user_columns, user_from_row};
use crate::sql::{query_all, query_count, query_optional};

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
#[instrument(skip(db), level = "debug")]
pub async fn get_user(db: &Database, username: &str) -> Result<DbUser> {
    find_user(db, username)
        .await?
        .ok_or_else(|| anyhow!("User not found: {}", username))
}

/// Find a user by username, returning `None` when no such user exists
///
/// Use this instead of `get_user` when a missing user is an expected outcome that the
/// caller must tell apart from a database failure.
///
/// # Errors
/// - If there's a database error during the query
///
#[instrument(skip(db), level = "debug")]
pub async fn find_user(db: &Database, username: &str) -> Result<Option<DbUser>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        concat!("SELECT ", user_columns!(), " FROM user WHERE username = ?1"),
        [normalize_username(username)],
        user_from_row,
    )
    .await
    .context(format!("Failed to query user from database: {}", username))
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
/// - If the id is not a user id
/// - If there's a database error during the query
///
#[instrument(skip(db), level = "debug")]
pub async fn get_user_by_id(db: &Database, id: RecordId) -> Result<Option<DbUser>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        concat!("SELECT ", user_columns!(), " FROM user WHERE id = ?1"),
        [key_of(&id, USER_TABLE)?],
        user_from_row,
    )
    .await
    .context(format!("Failed to query user by id: {:?}", id))
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
#[instrument(skip(db), level = "debug")]
pub async fn list_users(db: &Database) -> Result<Vec<DbUser>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!(
            "SELECT ",
            user_columns!(),
            " FROM user ORDER BY username ASC"
        ),
        (),
        user_from_row,
    )
    .await
    .context("Failed to query all users from database")
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
#[instrument(skip(db), level = "debug")]
pub async fn count_users(db: &Database) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(&conn, "SELECT count(*) FROM user", ())
        .await
        .context("Failed to count users from database")
}

/// Get a user by username for authentication purposes
///
/// This is the same as get_user but explicitly named for authentication use cases.
/// Returns the full user record including password_hash field needed for verification.
///
/// # Arguments
/// * `db` - Database connection
/// * `username` - The username to search for
///
/// # Returns
/// The DbUser if found, including password_hash for authentication
///
/// # Errors
/// - If the user is not found
/// - If there's a database error during the query
///
/// # Security Note
/// This function returns the password hash. Only use it for authentication purposes
/// and never expose the hash in API responses or logs.
#[instrument(skip(db), level = "debug")]
pub async fn get_user_for_auth(db: &Database, username: &str) -> Result<DbUser> {
    get_user(db, username).await
}
