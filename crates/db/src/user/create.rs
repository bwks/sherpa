use std::sync::Arc;
use anyhow::{Context, Result, anyhow};
use shared::auth::password::hash_password;
use shared::data::DbUser;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use surrealdb_types::Datetime;

/// Validate username format according to schema constraints
///
/// Rules:
/// - Minimum 3 characters
/// - Only alphanumeric characters plus @._-
fn validate_username(username: &str) -> Result<()> {
    // Check length
    if username.len() < 3 {
        return Err(anyhow!(
            "Username must be at least 3 characters long, got: {}",
            username.len()
        ));
    }

    // Check valid characters (alphanumeric + @._-)
    let valid_chars = username
        .chars()
        .all(|c| c.is_alphanumeric() || c == '@' || c == '.' || c == '_' || c == '-');

    if !valid_chars {
        return Err(anyhow!(
            "Username can only contain alphanumeric characters and @._- symbols. Got: '{}'",
            username
        ));
    }

    Ok(())
}

/// Create a new user in the database
///
/// # Arguments
/// * `db` - Database connection
/// * `username` - Username (min 3 chars, alphanumeric + @._-)
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
/// # Example
/// ```no_run
/// # use db::{connect, create_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let user = create_user(&db, "alice".to_string(), "SecurePass123!", false, vec![]).await?;
/// assert_eq!(user.username, "alice");
/// # Ok(())
/// # }
/// ```
pub async fn create_user(
    db: &Arc<Surreal<Client>>,
    username: String,
    password: &str,
    is_admin: bool,
    ssh_keys: Vec<String>,
) -> Result<DbUser> {
    // Validate username format
    validate_username(&username)?;

    // Hash the password (this also validates password strength)
    let password_hash = hash_password(password)?;

    // Generate current timestamp using SurrealDB's Datetime type
    let now = Datetime::default(); // Datetime::default() returns current time

    let db_user = DbUser {
        id: None,
        username: username.clone(),
        password_hash,
        is_admin,
        ssh_keys,
        created_at: now.clone(),
        updated_at: now,
    };

    // Create user record
    let created: Option<DbUser> = db
        .create("user")
        .content(db_user)
        .await
        .context(format!("Failed to create user: '{}'", username))?;

    created.ok_or_else(|| anyhow!("User was not created: '{}'", username))
}

/// Upsert a user (create if not exists, update if exists)
///
/// This uses the username as a unique identifier and will update the password and ssh_keys
/// if a user with that username already exists.
///
/// # Arguments
/// * `db` - Database connection
/// * `username` - Username (min 3 chars, alphanumeric + @._-)
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
/// # Example
/// ```no_run
/// # use db::{connect, upsert_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
///
/// // First call creates the user
/// let user1 = upsert_user(&db, "bob".to_string(), "Pass123!", false, vec!["key1".to_string()]).await?;
///
/// // Second call updates the existing user
/// let user2 = upsert_user(&db, "bob".to_string(), "NewPass123!", false, vec!["key1".to_string(), "key2".to_string()]).await?;
/// # Ok(())
/// # }
/// ```
pub async fn upsert_user(
    db: &Arc<Surreal<Client>>,
    username: String,
    password: &str,
    is_admin: bool,
    ssh_keys: Vec<String>,
) -> Result<DbUser> {
    // Validate username format
    validate_username(&username)?;

    // Hash the password (this also validates password strength)
    let password_hash = hash_password(password)?;

    // Generate current timestamp using SurrealDB's Datetime type
    let now = Datetime::default(); // Datetime::default() returns current time

    // Check if user exists to preserve created_at
    let existing_user = db
        .select::<Option<DbUser>>(("user", username.clone()))
        .await
        .ok()
        .flatten();

    let created_at = existing_user
        .map(|u| u.created_at)
        .unwrap_or_else(|| now.clone());

    // Upsert using username as the record ID
    let upserted: Option<DbUser> = db
        .upsert(("user", username.clone()))
        .content(DbUser {
            id: None,
            username: username.clone(),
            password_hash,
            is_admin,
            ssh_keys,
            created_at,
            updated_at: now,
        })
        .await
        .context(format!("Failed to upsert user '{}'", username))?;

    upserted.ok_or_else(|| anyhow!("User was not upserted: '{}'", username))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_username_valid() {
        assert!(validate_username("alice").is_ok());
        assert!(validate_username("bob123").is_ok());
        assert!(validate_username("user@example.com").is_ok());
        assert!(validate_username("test-user_01").is_ok());
        assert!(validate_username("a.b.c").is_ok());
    }

    #[test]
    fn test_validate_username_too_short() {
        let result = validate_username("ab");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("at least 3"));
    }

    #[test]
    fn test_validate_username_invalid_chars() {
        let result = validate_username("user name");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("alphanumeric"));

        let result = validate_username("user#name");
        assert!(result.is_err());

        let result = validate_username("user!name");
        assert!(result.is_err());
    }
}
