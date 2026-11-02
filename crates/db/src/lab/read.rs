use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use ipnet::{Ipv4Net, Ipv6Net};
use shared::data::{DbLab, RecordId};
use tracing::instrument;
use turso::Row;

use crate::Database;
use crate::persistence::{Cells, LAB_TABLE, USER_TABLE, key_of, lab_columns, lab_from_row};
use crate::sql::{query_all, query_count, query_optional};

/// Get a lab by its lab_id (business key)
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Returns
/// The DbLab record
///
/// # Errors
/// - If lab with lab_id not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn get_lab(db: &Database, lab_id: &str) -> Result<DbLab> {
    find_lab(db, lab_id)
        .await?
        .ok_or_else(|| anyhow!("Lab with lab_id not found: {}", lab_id))
}

/// Find a lab by its lab_id, returning `None` when no such lab exists
///
/// Use this instead of `get_lab` when a missing lab is an expected outcome that the
/// caller must tell apart from a database failure.
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn find_lab(db: &Database, lab_id: &str) -> Result<Option<DbLab>> {
    let conn = db.reader().await?;
    query_optional(
        &conn,
        concat!("SELECT ", lab_columns!(), " FROM lab WHERE lab_id = ?1"),
        [lab_id],
        lab_from_row,
    )
    .await
    .context(format!("Failed to query lab from database: {}", lab_id))
}

/// Get a lab by its RecordId (surrogate key)
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the lab
///
/// # Returns
/// The DbLab record
///
/// # Errors
/// - If lab with id not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn get_lab_by_id(db: &Database, id: RecordId) -> Result<DbLab> {
    let conn = db.reader().await?;
    let lab = query_optional(
        &conn,
        concat!("SELECT ", lab_columns!(), " FROM lab WHERE id = ?1"),
        [key_of(&id, LAB_TABLE)?],
        lab_from_row,
    )
    .await
    .context(format!("Failed to get lab by id: {:?}", id))?;

    lab.ok_or_else(|| anyhow!("Lab not found with id: {:?}", id))
}

/// Get a lab by name and user (unique constraint)
///
/// # Arguments
/// * `db` - Database connection
/// * `name` - Lab name
/// * `user_id` - Owner's RecordId
///
/// # Returns
/// The DbLab record
///
/// # Errors
/// - If lab not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn get_lab_by_name_and_user(
    db: &Database,
    name: &str,
    user_id: RecordId,
) -> Result<DbLab> {
    let conn = db.reader().await?;
    let db_lab = query_optional(
        &conn,
        concat!(
            "SELECT ",
            lab_columns!(),
            " FROM lab WHERE name = ?1 AND user = ?2"
        ),
        [name.to_string(), key_of(&user_id, USER_TABLE)?],
        lab_from_row,
    )
    .await
    .context(format!("Failed to query lab by name and user: {}", name))?;

    db_lab.ok_or_else(|| anyhow!("Lab not found with name '{}' for user: {:?}", name, user_id))
}

/// List all labs in the database
///
/// # Arguments
/// * `db` - Database connection
///
/// # Returns
/// Vector of all DbLab records
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn list_labs(db: &Database) -> Result<Vec<DbLab>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!("SELECT ", lab_columns!(), " FROM lab"),
        (),
        lab_from_row,
    )
    .await
    .context("Failed to list labs from database")
}

/// List all labs owned by a specific user
///
/// # Arguments
/// * `db` - Database connection
/// * `user_id` - Owner's RecordId
///
/// # Returns
/// Vector of DbLab records owned by the user
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn list_labs_by_user(db: &Database, user_id: RecordId) -> Result<Vec<DbLab>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        concat!("SELECT ", lab_columns!(), " FROM lab WHERE user = ?1"),
        [key_of(&user_id, USER_TABLE)?],
        lab_from_row,
    )
    .await
    .context(format!("Failed to list labs for user: {:?}", user_id))
}

/// Count total number of labs in the database
///
/// # Arguments
/// * `db` - Database connection
///
/// # Returns
/// Total count of labs
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn count_labs(db: &Database) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(&conn, "SELECT count(*) FROM lab", ())
        .await
        .context("Failed to count labs")
}

/// Count number of labs owned by a specific user
///
/// # Arguments
/// * `db` - Database connection
/// * `user_id` - Owner's RecordId
///
/// # Returns
/// Count of labs owned by the user
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn count_labs_by_user(db: &Database, user_id: RecordId) -> Result<usize> {
    let conn = db.reader().await?;
    query_count(
        &conn,
        "SELECT count(*) FROM lab WHERE user = ?1",
        [key_of(&user_id, USER_TABLE)?],
    )
    .await
    .context("Failed to count labs for user")
}

/// Get all loopback networks currently allocated to labs.
///
/// Returns a vector of `Ipv4Net` representing the loopback subnets
/// in use by existing labs. Used when allocating a new loopback subnet
/// to avoid collisions.
#[instrument(skip(db), level = "debug")]
pub async fn get_used_loopback_networks(db: &Database) -> Result<Vec<Ipv4Net>> {
    let conn = db.reader().await?;
    let rows = query_all(
        &conn,
        "SELECT lab_id, loopback_network FROM lab",
        (),
        lab_network_from_row,
    )
    .await
    .context("Failed to query labs for loopback networks")?;

    let mut networks = Vec::new();
    for (lab_id, network) in rows {
        let net = Ipv4Net::from_str(&network).with_context(|| {
            format!(
                "Failed to parse loopback_network '{}' for lab '{}'",
                network, lab_id
            )
        })?;
        networks.push(net);
    }
    Ok(networks)
}

/// Get all management networks currently allocated to labs.
///
/// Returns a vector of `Ipv4Net` representing the management subnets
/// in use by existing labs. Used when allocating a new management subnet
/// to avoid collisions.
#[instrument(skip(db), level = "debug")]
pub async fn get_used_management_networks(db: &Database) -> Result<Vec<Ipv4Net>> {
    let conn = db.reader().await?;
    let rows = query_all(
        &conn,
        "SELECT lab_id, management_network FROM lab",
        (),
        lab_network_from_row,
    )
    .await
    .context("Failed to query labs for management networks")?;

    let mut networks = Vec::new();
    for (lab_id, network) in rows {
        let net = Ipv4Net::from_str(&network).with_context(|| {
            format!(
                "Failed to parse management_network '{}' for lab '{}'",
                network, lab_id
            )
        })?;
        networks.push(net);
    }
    Ok(networks)
}

/// Get all IPv6 management networks currently allocated to labs.
///
/// Returns a vector of `Ipv6Net` representing the IPv6 management subnets
/// in use by existing labs. Labs without IPv6 are skipped.
#[instrument(skip(db), level = "debug")]
pub async fn get_used_ipv6_management_networks(db: &Database) -> Result<Vec<Ipv6Net>> {
    let conn = db.reader().await?;
    let rows = query_all(
        &conn,
        "SELECT lab_id, management_network_v6 FROM lab WHERE management_network_v6 IS NOT NULL",
        (),
        lab_network_from_row,
    )
    .await
    .context("Failed to query labs for IPv6 management networks")?;

    let mut networks = Vec::new();
    for (lab_id, network) in rows {
        let net = Ipv6Net::from_str(&network).with_context(|| {
            format!(
                "Failed to parse management_network_v6 '{}' for lab '{}'",
                network, lab_id
            )
        })?;
        networks.push(net);
    }
    Ok(networks)
}

/// Get all IPv6 loopback networks currently allocated to labs.
///
/// Returns a vector of `Ipv6Net` representing the IPv6 loopback subnets
/// in use by existing labs. Labs without IPv6 are skipped.
#[instrument(skip(db), level = "debug")]
pub async fn get_used_ipv6_loopback_networks(db: &Database) -> Result<Vec<Ipv6Net>> {
    let conn = db.reader().await?;
    let rows = query_all(
        &conn,
        "SELECT lab_id, loopback_network_v6 FROM lab WHERE loopback_network_v6 IS NOT NULL",
        (),
        lab_network_from_row,
    )
    .await
    .context("Failed to query labs for IPv6 loopback networks")?;

    let mut networks = Vec::new();
    for (lab_id, network) in rows {
        let net = Ipv6Net::from_str(&network).with_context(|| {
            format!(
                "Failed to parse loopback_network_v6 '{}' for lab '{}'",
                network, lab_id
            )
        })?;
        networks.push(net);
    }
    Ok(networks)
}

/// A lab's `lab_id` and one network column, from `SELECT lab_id, <network> FROM lab`.
fn lab_network_from_row(row: &Row) -> Result<(String, String)> {
    let mut cells = Cells::new(row);
    Ok((cells.text("lab_id")?, cells.text("network")?))
}
