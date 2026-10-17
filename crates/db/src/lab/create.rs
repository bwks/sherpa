use anyhow::{Context, Result, anyhow};
use shared::data::{DbLab, DbUser, LabState};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;
use tracing::instrument;

use crate::helpers::get_user_id;
use crate::persistence::{LabRow, to_surreal_id};

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
    db: &Arc<Surreal<Client>>,
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
    let created: Option<LabRow> = db
        .create("lab")
        .content(LabRow::try_from(&lab)?)
        .await
        .context(format!("Failed to create lab: '{}'", name))?;

    created
        .map(DbLab::try_from)
        .transpose()?
        .ok_or_else(|| anyhow!("Lab was not created: '{}'", name))
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
pub async fn upsert_lab(db: &Arc<Surreal<Client>>, lab: DbLab) -> Result<DbLab> {
    // Validate lab_id format
    validate_lab_id(&lab.lab_id)?;

    let row = LabRow::try_from(&lab)?;
    let result: Option<LabRow> = if let Some(id) = &lab.id {
        // Update existing lab
        db.update(to_surreal_id(id))
            .content(row)
            .await
            .context(format!("Failed to update lab: '{}'", lab.name))?
    } else {
        // Create new lab
        db.create("lab")
            .content(row)
            .await
            .context(format!("Failed to create lab: '{}'", lab.name))?
    };

    result
        .map(DbLab::try_from)
        .transpose()?
        .ok_or_else(|| anyhow!("Lab upsert failed: '{}'", lab.name))
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
