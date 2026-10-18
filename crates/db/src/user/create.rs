use anyhow::{Context, Result, anyhow};
use jiff::Timestamp;
use shared::auth::password::hash_password;
use shared::data::DbUser;
use shared::konst::ADMIN_USERNAME;
use shared::util::normalize_username;
use shared::validators::validate_username;
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use tracing::instrument;

use crate::persistence::UserRow;

/// Create a new user in the database
///
/// # Arguments
/// * `db` - Database connection
/// * `username` - Username, normalised to lower case (see `shared::validators::validate_username`)
/// * `password` - Plain text password (will be hashed with Argon2id)
/// * `is_admin` - Whether the user should have admin privileges
/// * `ssh_keys` - Optional list of SSH public keys
///
/// # Returns
/// The created DbUser with assigned ID
///
/// # Errors
/// - If username validation fails
/// - If password validation fails (see shared::auth::password)
/// - If username already exists (unique constraint violation)
/// - If there's a database error during creation
///
#[instrument(skip(db, password), level = "debug")]
pub async fn create_user(
    db: &Arc<Surreal<Client>>,
    username: String,
    password: &str,
    is_admin: bool,
    ssh_keys: Vec<String>,
) -> Result<DbUser> {
    let username = normalize_username(&username);
    validate_username(&username)?;
    insert_user(db, username, password, is_admin, ssh_keys).await
}

/// Create the built-in `admin` account.
///
/// `admin` is reserved, so `create_user` refuses it. Only server bootstrap (`sherpad init` and
/// startup seeding) calls this.
///
/// # Errors
/// - If password validation fails
/// - If `admin` already exists (unique constraint violation)
/// - If there's a database error during creation
#[instrument(skip(db, password), level = "debug")]
pub async fn create_admin_user(db: &Arc<Surreal<Client>>, password: &str) -> Result<DbUser> {
    insert_user(db, ADMIN_USERNAME.to_string(), password, true, vec![]).await
}

async fn insert_user(
    db: &Arc<Surreal<Client>>,
    username: String,
    password: &str,
    is_admin: bool,
    ssh_keys: Vec<String>,
) -> Result<DbUser> {
    // Hash the password (this also validates password strength)
    let password_hash = hash_password(password)?;

    let now = Timestamp::now();

    let db_user = DbUser {
        id: None,
        username: username.clone(),
        password_hash,
        is_admin,
        ssh_keys,
        created_at: now,
        updated_at: now,
    };

    // Create user record
    let row = UserRow::try_from(&db_user)?;
    let created: Option<UserRow> = db
        .create("user")
        .content(row)
        .await
        .context(format!("Failed to create user: '{}'", username))?;

    created
        .map(DbUser::try_from)
        .transpose()?
        .ok_or_else(|| anyhow!("User was not created: '{}'", username))
}

/// Upsert a user (create if not exists, update if exists)
///
/// This uses the username as a unique identifier and will update the password and ssh_keys
/// if a user with that username already exists.
///
/// # Arguments
/// * `db` - Database connection
/// * `username` - Username, normalised to lower case (see `shared::validators::validate_username`)
/// * `password` - Plain text password (will be hashed with Argon2id)
/// * `is_admin` - Whether the user should have admin privileges
/// * `ssh_keys` - List of SSH public keys (will replace existing keys)
///
/// # Returns
/// The created or updated DbUser
///
/// # Errors
/// - If username validation fails
/// - If password validation fails
/// - If there's a database error during the operation
///
#[instrument(skip(db, password), level = "debug")]
pub async fn upsert_user(
    db: &Arc<Surreal<Client>>,
    username: String,
    password: &str,
    is_admin: bool,
    ssh_keys: Vec<String>,
) -> Result<DbUser> {
    let username = normalize_username(&username);
    validate_username(&username)?;

    // Hash the password (this also validates password strength)
    let password_hash = hash_password(password)?;

    let now = Timestamp::now();

    // Check if user exists to preserve created_at
    let existing_user = db
        .select::<Option<UserRow>>(("user", username.clone()))
        .await
        .ok()
        .flatten();

    let created_at = existing_user
        .map(DbUser::try_from)
        .transpose()?
        .map_or(now, |user| user.created_at);

    let user = DbUser {
        id: None,
        username: username.clone(),
        password_hash,
        is_admin,
        ssh_keys,
        created_at,
        updated_at: now,
    };
    let row = UserRow::try_from(&user)?;

    // Upsert using username as the record ID
    let upserted: Option<UserRow> = db
        .upsert(("user", username.clone()))
        .content(row)
        .await
        .context(format!("Failed to upsert user '{}'", username))?;

    upserted
        .map(DbUser::try_from)
        .transpose()?
        .ok_or_else(|| anyhow!("User was not upserted: '{}'", username))
}
