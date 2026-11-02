use anyhow::{Context, Result};
use tracing::instrument;

use shared::konst::ADMIN_USERNAME;

use crate::Database;
use crate::user::{create_admin_user, find_user};

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
pub async fn seed_admin_user(db: &Database, password: &str) -> Result<bool> {
    // Check if admin user already exists
    // A failed lookup is an error, not a missing admin: seeding must not mask a broken database.
    match find_user(db, ADMIN_USERNAME).await? {
        Some(_) => {
            tracing::debug!("Admin user seeding skipped (admin user already exists)");
            return Ok(false);
        }
        None => {
            tracing::debug!("Admin user not found, creating it");
        }
    }

    // Create admin user
    create_admin_user(db, password)
        .await
        .context("Failed to create admin user")?;

    tracing::info!("Admin user created successfully");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user::{count_users, create_user, get_user};

    #[tokio::test]
    async fn test_seed_admin_user_creates_when_empty() -> Result<()> {
        let db = crate::open_in_memory().await?;
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
    async fn test_seed_admin_user_creates_when_other_users_exist() -> Result<()> {
        let db = crate::open_in_memory().await?;
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
    async fn test_seed_admin_user_validates_password() -> Result<()> {
        let db = crate::open_in_memory().await?;
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
    async fn test_seed_admin_user_idempotent() -> Result<()> {
        let db = crate::open_in_memory().await?;
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
