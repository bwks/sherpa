use anyhow::{Context, Result};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use tracing::instrument;

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
#[instrument(skip(db, password), level = "debug")]
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
    use shared::konst::SHERPA_PASSWORD;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_db_port() -> u16 {
        std::env::var("SHERPA_DEV_DB_PORT")
            .unwrap_or_else(|_| "42069".to_string())
            .parse()
            .expect("SHERPA_DEV_DB_PORT must be a valid port number")
    }

    /// Generate a unique namespace per test invocation to avoid index collisions
    fn unique_ns(test_name: &str) -> String {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos();
        format!("test_ns_{ts}_{test_name}")
    }

    #[tokio::test]
    #[ignore] // Requires running SurrealDB instance
    async fn test_seed_admin_user_creates_when_empty() -> Result<()> {
        let db = crate::connect(
            "localhost",
            test_db_port(),
            &unique_ns("seed_admin_empty"),
            "test_db",
            SHERPA_PASSWORD,
        )
        .await?;
        crate::schema::apply_schema(&db).await?;

        let created = seed_admin_user(&db, "AdminPass123!").await?;
        assert!(created, "Should create admin user when database is empty");

        // Verify admin user exists
        let admin = get_user(&db, "admin").await?;
        assert_eq!(admin.username, "admin");
        assert!(admin.is_admin, "User should have admin privileges");

        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires running SurrealDB instance
    async fn test_seed_admin_user_creates_when_other_users_exist() -> Result<()> {
        let db = crate::connect(
            "localhost",
            test_db_port(),
            &unique_ns("seed_admin_others"),
            "test_db",
            SHERPA_PASSWORD,
        )
        .await?;
        crate::schema::apply_schema(&db).await?;

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

        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires running SurrealDB instance
    async fn test_seed_admin_user_validates_password() -> Result<()> {
        let db = crate::connect(
            "localhost",
            test_db_port(),
            &unique_ns("seed_admin_validate"),
            "test_db",
            SHERPA_PASSWORD,
        )
        .await?;
        crate::schema::apply_schema(&db).await?;

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
        let db = crate::connect(
            "localhost",
            test_db_port(),
            &unique_ns("seed_admin_idem"),
            "test_db",
            SHERPA_PASSWORD,
        )
        .await?;
        crate::schema::apply_schema(&db).await?;

        // First call should create
        let created1 = seed_admin_user(&db, "AdminPass123!").await?;
        assert!(created1, "First call should create admin user");

        // Second call should skip
        let created2 = seed_admin_user(&db, "AdminPass123!").await?;
        assert!(!created2, "Second call should skip (user exists)");

        // Should still have exactly 1 user
        let user_count = count_users(&db).await?;
        assert_eq!(user_count, 1, "Should have exactly one user");

        Ok(())
    }
}
