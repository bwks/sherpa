use anyhow::{Context, Result};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::user::{create_user, get_user};

/// Seed the database with an admin user
///
/// This function creates an admin user with the provided password if no user
/// with username "admin" exists.
/// It's designed to run during initial server setup to bootstrap the authentication system.
///
/// # Arguments
/// * `db` - Database connection
/// * `password` - Admin user password (must meet security requirements)
///
/// # Returns
/// * `Ok(true)` if admin user was created
/// * `Ok(false)` if admin user already exists
/// * `Err` if password validation fails or database operation fails
///
/// # Security
/// This function should only be called during server initialization with a password
/// provided via secure means (environment variable, not config file).
///
/// # Example
/// ```no_run
/// # use db::{connect, seed_admin_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let password = std::env::var("SHERPA_ADMIN_PASSWORD")?;
///
/// if seed_admin_user(&db, &password).await? {
///     println!("Admin user created successfully");
/// } else {
///     println!("Admin user already exists");
/// }
/// # Ok(())
/// # }
/// ```
pub async fn seed_admin_user(db: &Arc<Surreal<Client>>, password: &str) -> Result<bool> {
    // Check if admin user already exists
    match get_user(db, "admin").await {
        Ok(_) => {
            tracing::debug!("Admin user seeding skipped (admin user already exists)");
            return Ok(false);
        }
        Err(_) => {
            // Admin user doesn't exist, proceed with creation
        }
    }

    // Create admin user
    create_user(db, "admin".to_string(), password, true, vec![])
        .await
        .context("Failed to create admin user")?;

    tracing::info!("Admin user created successfully");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user::{count_users, get_user};

    #[tokio::test]
    #[ignore] // Requires running SurrealDB instance
    async fn test_seed_admin_user_creates_when_empty() -> Result<()> {
        let db = crate::connect("localhost", 8000, "test_seed_admin", "test_db").await?;
        crate::schema::apply_schema(&db).await?;

        // Clean up any existing users
        let _: Vec<crate::DbUser> = db.delete("user").await?;

        let created = seed_admin_user(&db, "AdminPass123!").await?;
        assert!(created, "Should create admin user when database is empty");

        // Verify admin user exists
        let admin = get_user(&db, "admin").await?;
        assert_eq!(admin.username, "admin");
        assert!(admin.is_admin, "User should have admin privileges");

        // Clean up
        let _: Vec<crate::DbUser> = db.delete("user").await?;

        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires running SurrealDB instance
    async fn test_seed_admin_user_creates_when_other_users_exist() -> Result<()> {
        let db = crate::connect("localhost", 8000, "test_seed_admin_skip", "test_db").await?;
        crate::schema::apply_schema(&db).await?;

        // Clean up any existing users
        let _: Vec<crate::DbUser> = db.delete("user").await?;

        // Create a regular user first
        create_user(&db, "alice".to_string(), "AlicePass123!", false, vec![]).await?;

        // Seed admin user should still work (because "admin" doesn't exist)
        let created = seed_admin_user(&db, "AdminPass123!").await?;
        assert!(
            created,
            "Should create admin user even when other users exist"
        );

        // Verify admin user was created
        let admin = get_user(&db, "admin").await?;
        assert_eq!(admin.username, "admin");
        assert!(admin.is_admin, "User should have admin privileges");

        // Clean up
        let _: Vec<crate::DbUser> = db.delete("user").await?;

        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires running SurrealDB instance
    async fn test_seed_admin_user_validates_password() -> Result<()> {
        let db = crate::connect("localhost", 8000, "test_seed_admin_validate", "test_db").await?;
        crate::schema::apply_schema(&db).await?;

        // Clean up any existing users
        let _: Vec<crate::DbUser> = db.delete("user").await?;

        // Try with invalid password
        let result = seed_admin_user(&db, "weak").await;
        assert!(result.is_err(), "Should fail with weak password");

        // Verify no user was created
        let user_count = count_users(&db).await?;
        assert_eq!(
            user_count, 0,
            "No users should be created with invalid password"
        );

        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires running SurrealDB instance
    async fn test_seed_admin_user_idempotent() -> Result<()> {
        let db = crate::connect("localhost", 8000, "test_seed_admin_idem", "test_db").await?;
        crate::schema::apply_schema(&db).await?;

        // Clean up any existing users
        let _: Vec<crate::DbUser> = db.delete("user").await?;

        // First call should create
        let created1 = seed_admin_user(&db, "AdminPass123!").await?;
        assert!(created1, "First call should create admin user");

        // Second call should skip
        let created2 = seed_admin_user(&db, "AdminPass123!").await?;
        assert!(!created2, "Second call should skip (user exists)");

        // Should still have exactly 1 user
        let user_count = count_users(&db).await?;
        assert_eq!(user_count, 1, "Should have exactly one user");

        // Clean up
        let _: Vec<crate::DbUser> = db.delete("user").await?;

        Ok(())
    }
}
