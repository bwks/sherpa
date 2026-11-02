use anyhow::{Context, Result, anyhow};
use shared::data::{DbLab, DbUser, LabState};
use tracing::instrument;
use turso::Connection;

use crate::Database;
use crate::helpers::get_user_id;
use crate::persistence::{LAB_TABLE, key_of, lab_columns, lab_from_row, lab_values, new_key};
use crate::sql::query_optional;

/// The subnets and addresses allocated to one lab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabNetworks {
    pub loopback_network: String,
    pub management_network: String,
    pub gateway_ipv4: String,
    pub router_ipv4: String,
    pub management_network_v6: Option<String>,
    pub loopback_network_v6: Option<String>,
    pub gateway_ipv6: Option<String>,
    pub router_ipv6: Option<String>,
}

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
#[instrument(level = "debug")]
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

/// Create a new lab holding all of its allocated subnets in one insert.
///
/// The unique subnet indexes make this the reservation: if another lab already holds
/// any of the subnets, the insert fails and nothing is created.
///
/// # Errors
/// - If lab_id validation fails (not 8 characters)
/// - If lab_id, (name, user) or any allocated subnet is already in use
/// - If there's a database error during creation
#[instrument(skip(db), level = "debug")]
pub async fn create_lab(
    db: &Database,
    name: &str,
    lab_id: &str,
    user: &DbUser,
    networks: &LabNetworks,
) -> Result<DbLab> {
    // Validate lab_id format
    validate_lab_id(lab_id)?;

    // Get user's record ID
    let user_id = get_user_id(user)?;

    // Create lab record
    let lab = DbLab {
        id: None,
        lab_id: lab_id.to_string(),
        name: name.to_string(),
        user: user_id,
        loopback_network: networks.loopback_network.clone(),
        management_network: networks.management_network.clone(),
        gateway_ipv4: networks.gateway_ipv4.clone(),
        router_ipv4: networks.router_ipv4.clone(),
        tailscale_container_id: None,
        management_network_v6: networks.management_network_v6.clone(),
        gateway_ipv6: networks.gateway_ipv6.clone(),
        router_ipv6: networks.router_ipv6.clone(),
        loopback_network_v6: networks.loopback_network_v6.clone(),
        status: LabState::default(),
    };
    let writer = db.writer().await?;
    let created = insert_lab(&writer.conn, &lab)
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
#[instrument(skip(db), level = "debug")]
pub async fn upsert_lab(db: &Database, lab: DbLab) -> Result<DbLab> {
    // Validate lab_id format
    validate_lab_id(&lab.lab_id)?;

    let writer = db.writer().await?;
    let result = if let Some(id) = &lab.id {
        // Update existing lab
        query_optional(
            &writer.conn,
            UPDATE_LAB,
            lab_values(&lab, &key_of(id, LAB_TABLE)?)?,
            lab_from_row,
        )
        .await
        .context(format!("Failed to update lab: '{}'", lab.name))?
    } else {
        // Create new lab
        insert_lab(&writer.conn, &lab)
            .await
            .context(format!("Failed to create lab: '{}'", lab.name))?
    };

    result.ok_or_else(|| anyhow!("Lab upsert failed: '{}'", lab.name))
}

/// Replace every column of a lab except its id, returning the stored lab. Used by
/// `upsert_lab` and `update_lab`.
pub(crate) const UPDATE_LAB: &str = concat!(
    "UPDATE lab SET lab_id = ?2, name = ?3, user = ?4, loopback_network = ?5, \
     management_network = ?6, gateway_ipv4 = ?7, router_ipv4 = ?8, \
     tailscale_container_id = ?9, management_network_v6 = ?10, gateway_ipv6 = ?11, \
     router_ipv6 = ?12, loopback_network_v6 = ?13, status = ?14 WHERE id = ?1 RETURNING ",
    lab_columns!()
);

/// Insert `lab` under a new key, returning the stored lab.
async fn insert_lab(conn: &Connection, lab: &DbLab) -> Result<Option<DbLab>> {
    query_optional(
        conn,
        concat!(
            "INSERT INTO lab (",
            lab_columns!(),
            ") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) RETURNING ",
            lab_columns!()
        ),
        lab_values(lab, &new_key())?,
        lab_from_row,
    )
    .await
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
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exactly 8 characters")
        );
    }

    #[test]
    fn test_validate_lab_id_too_long() {
        let result = validate_lab_id("lab-00001");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exactly 8 characters")
        );
    }

    #[test]
    fn test_validate_lab_id_invalid_chars() {
        let result = validate_lab_id("lab_0001");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("alphanumeric characters and hyphens")
        );
    }

    #[test]
    fn test_validate_lab_id_spaces() {
        let result = validate_lab_id("lab 0001");
        assert!(result.is_err());
    }
}
