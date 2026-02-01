use anyhow::{anyhow, Context, Result};
use data::{DbLab, DbUser};
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::helpers::get_user_id;

/// Validate lab_id format
///
/// Requirements:
/// - Must be exactly 8 characters long
/// - Should contain alphanumeric characters and hyphens
///
/// # Arguments
/// * `lab_id` - The lab_id string to validate
///
/// # Returns
/// * `Ok(())` if valid
/// * `Err` with descriptive message if invalid
pub fn validate_lab_id(lab_id: &str) -> Result<()> {
    // Check length
    if lab_id.len() != 8 {
        return Err(anyhow!(
            "lab_id must be exactly 8 characters long, got {} characters",
            lab_id.len()
        ));
    }

    // Check for valid characters (alphanumeric + hyphen)
    if !lab_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(anyhow!(
            "lab_id must contain only alphanumeric characters and hyphens"
        ));
    }

    Ok(())
}

/// Create a new lab in the database
///
/// # Arguments
/// * `db` - Database connection
/// * `name` - Lab name (can be duplicated across different users)
/// * `lab_id` - Unique lab identifier (must be exactly 8 characters)
/// * `user` - Lab owner (DbUser record with id)
///
/// # Returns
/// The created DbLab with assigned ID
///
/// # Errors
/// - If lab_id validation fails (not 8 characters)
/// - If lab_id already exists (unique constraint violation)
/// - If (name, user) combination already exists (unique constraint violation)
/// - If there's a database error during creation
///
/// # Example
/// ```no_run
/// # use db::{connect, create_lab, create_user};
/// # use konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let user = create_user(&db, "alice".to_string(), vec![]).await?;
/// let lab = create_lab(&db, "My Lab", "lab-0001", &user).await?;
/// assert_eq!(lab.name, "My Lab");
/// assert_eq!(lab.lab_id, "lab-0001");
/// # Ok(())
/// # }
/// ```
pub async fn create_lab(
    db: &Surreal<Client>,
    name: &str,
    lab_id: &str,
    user: &DbUser,
) -> Result<DbLab> {
    // Validate lab_id format
    validate_lab_id(lab_id)?;

    // Get user's record ID
    let user_id = get_user_id(user)?;

    // Create lab record
    let created: Option<DbLab> = db
        .create("lab")
        .content(DbLab {
            id: None,
            lab_id: lab_id.to_string(),
            name: name.to_string(),
            user: user_id,
        })
        .await
        .context(format!("Failed to create lab: '{}'", name))?;

    created.ok_or_else(|| anyhow!("Lab was not created: '{}'", name))
}

/// Create a new lab or update existing lab (upsert operation)
///
/// If a lab with the same `id` exists, it will be updated.
/// If no `id` is provided or lab doesn't exist, a new lab will be created.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab` - DbLab struct with all fields populated
///
/// # Returns
/// The created or updated DbLab
///
/// # Errors
/// - If lab_id validation fails
/// - If unique constraints are violated
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, create_user, upsert_lab};
/// # use data::{DbLab, RecordId};
/// # use konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let user = create_user(&db, "alice".to_string(), vec![]).await?;
/// let user_id = user.id.unwrap();
///
/// let lab = DbLab {
///     id: None,
///     lab_id: "lab-0001".to_string(),
///     name: "Updated Lab".to_string(),
///     user: user_id,
/// };
/// let result = upsert_lab(&db, lab).await?;
/// # Ok(())
/// # }
/// ```
pub async fn upsert_lab(db: &Surreal<Client>, lab: DbLab) -> Result<DbLab> {
    // Validate lab_id format
    validate_lab_id(&lab.lab_id)?;

    let result: Option<DbLab> = if let Some(id) = &lab.id {
        // Update existing lab
        db.update(id.clone())
            .content(lab.clone())
            .await
            .context(format!("Failed to update lab: '{}'", lab.name))?
    } else {
        // Create new lab
        db.create("lab")
            .content(lab.clone())
            .await
            .context(format!("Failed to create lab: '{}'", lab.name))?
    };

    result.ok_or_else(|| anyhow!("Lab upsert failed: '{}'", lab.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_lab_id_valid() {
        assert!(validate_lab_id("lab-0001").is_ok());
        assert!(validate_lab_id("12345678").is_ok());
        assert!(validate_lab_id("abc-defg").is_ok());
        assert!(validate_lab_id("TEST-123").is_ok());
    }

    #[test]
    fn test_validate_lab_id_too_short() {
        let result = validate_lab_id("lab-01");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exactly 8 characters"));
    }

    #[test]
    fn test_validate_lab_id_too_long() {
        let result = validate_lab_id("lab-00001");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exactly 8 characters"));
    }

    #[test]
    fn test_validate_lab_id_invalid_chars() {
        let result = validate_lab_id("lab_0001");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("alphanumeric characters and hyphens"));
    }

    #[test]
    fn test_validate_lab_id_spaces() {
        let result = validate_lab_id("lab 0001");
        assert!(result.is_err());
    }
}
