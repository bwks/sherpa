use anyhow::{Context, Result, anyhow};
use data::DbUser;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

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
/// * `ssh_keys` - Optional list of SSH public keys
///
/// # Returns
/// The created DbUser with assigned ID
///
/// # Errors
/// - If username validation fails
/// - If username already exists (unique constraint violation)
/// - If there's a database error during creation
///
/// # Example
/// ```no_run
/// # use db::{connect, create_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let user = create_user(&db, "alice".to_string(), vec![]).await?;
/// assert_eq!(user.username, "alice");
/// # Ok(())
/// # }
/// ```
pub async fn create_user(
    db: &Surreal<Client>,
    username: String,
    ssh_keys: Vec<String>,
) -> Result<DbUser> {
    // Validate username format
    validate_username(&username)?;

    // Create user record
    let created: Option<DbUser> = db
        .create("user")
        .content(DbUser {
            id: None,
            username: username.clone(),
            ssh_keys,
        })
        .await
        .context(format!("Failed to create user: '{}'", username))?;

    created.ok_or_else(|| anyhow!("User was not created: '{}'", username))
}

/// Upsert a user (create if not exists, update if exists)
///
/// This uses the username as a unique identifier and will update the ssh_keys
/// if a user with that username already exists.
///
/// # Arguments
/// * `db` - Database connection
/// * `username` - Username (min 3 chars, alphanumeric + @._-)
/// * `ssh_keys` - List of SSH public keys (will replace existing keys)
///
/// # Returns
/// The created or updated DbUser
///
/// # Errors
/// - If username validation fails
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
/// let user1 = upsert_user(&db, "bob".to_string(), vec!["key1".to_string()]).await?;
///
/// // Second call updates the existing user
/// let user2 = upsert_user(&db, "bob".to_string(), vec!["key1".to_string(), "key2".to_string()]).await?;
/// # Ok(())
/// # }
/// ```
pub async fn upsert_user(
    db: &Surreal<Client>,
    username: String,
    ssh_keys: Vec<String>,
) -> Result<DbUser> {
    // Validate username format
    validate_username(&username)?;

    // Upsert using username as the record ID
    let upserted: Option<DbUser> = db
        .upsert(("user", username.clone()))
        .content(DbUser {
            id: None,
            username: username.clone(),
            ssh_keys,
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
