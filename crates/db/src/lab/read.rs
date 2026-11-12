use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use ipnet::{Ipv4Net, Ipv6Net};
use shared::data::{DbLab, RecordId};
use tracing::instrument;
use turso::Row;

use crate::Database;
use crate::persistence::{Cells, USER_TABLE, key_of, lab_from_row};
use crate::queries;
use crate::sql::{Query, query_all, query_count, query_optional};

/// Get a lab by its `lab_id` (business key)
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique `lab_id` string
///
/// # Returns
/// The `DbLab` record
///
/// # Errors
/// - If lab with `lab_id` not found
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn get_lab(db: &Database, lab_id: &str) -> Result<DbLab> {
    find_lab(db, lab_id)
        .await?
        .ok_or_else(|| anyhow!("Lab with lab_id not found: {lab_id}"))
}

/// Find a lab by its `lab_id`, returning `None` when no such lab exists
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
        queries::lab::FIND_BY_LAB_ID.bind([lab_id.into()]),
        lab_from_row,
    )
    .await
    .context(format!("Failed to query lab from database: {lab_id}"))
}

/// List all labs in the database
///
/// # Arguments
/// * `db` - Database connection
///
/// # Returns
/// Vector of all `DbLab` records
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn list_labs(db: &Database) -> Result<Vec<DbLab>> {
    let conn = db.reader().await?;
    query_all(&conn, queries::lab::LIST.bind([]), lab_from_row)
        .await
        .context("Failed to list labs from database")
}

/// List all labs owned by a specific user
///
/// # Arguments
/// * `db` - Database connection
/// * `user_id` - Owner's `RecordId`
///
/// # Returns
/// Vector of `DbLab` records owned by the user
///
/// # Errors
/// - If there's a database error
///
#[instrument(skip(db), level = "debug")]
pub async fn list_labs_by_user(db: &Database, user_id: RecordId) -> Result<Vec<DbLab>> {
    let conn = db.reader().await?;
    query_all(
        &conn,
        queries::lab::LIST_BY_USER.bind([key_of(&user_id, USER_TABLE)?.into()]),
        lab_from_row,
    )
    .await
    .context(format!("Failed to list labs for user: {user_id:?}"))
}

/// Count number of labs owned by a specific user
///
/// # Arguments
/// * `db` - Database connection
/// * `user_id` - Owner's `RecordId`
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
        queries::lab::COUNT_BY_USER.bind([key_of(&user_id, USER_TABLE)?.into()]),
    )
    .await
    .context("Failed to count labs for user")
}

/// Get all loopback networks currently allocated to labs.
///
/// Returns a vector of `Ipv4Net` representing the loopback subnets
/// in use by existing labs. Used when allocating a new loopback subnet
/// to avoid collisions.
///
/// # Errors
///
/// Fails when the labs cannot be read, or a lab holds a network that does not parse.
#[instrument(skip(db), level = "debug")]
pub async fn get_used_loopback_networks(db: &Database) -> Result<Vec<Ipv4Net>> {
    used_networks(db, queries::lab::LOOPBACK_NETWORKS, "loopback_network").await
}

/// Get all management networks currently allocated to labs.
///
/// Returns a vector of `Ipv4Net` representing the management subnets
/// in use by existing labs. Used when allocating a new management subnet
/// to avoid collisions.
///
/// # Errors
///
/// Fails when the labs cannot be read, or a lab holds a network that does not parse.
#[instrument(skip(db), level = "debug")]
pub async fn get_used_management_networks(db: &Database) -> Result<Vec<Ipv4Net>> {
    used_networks(db, queries::lab::MANAGEMENT_NETWORKS, "management_network").await
}

/// Get all IPv6 management networks currently allocated to labs.
///
/// Returns a vector of `Ipv6Net` representing the IPv6 management subnets
/// in use by existing labs. Labs without IPv6 are skipped.
///
/// # Errors
///
/// Fails when the labs cannot be read, or a lab holds a network that does not parse.
#[instrument(skip(db), level = "debug")]
pub async fn get_used_ipv6_management_networks(db: &Database) -> Result<Vec<Ipv6Net>> {
    used_networks(
        db,
        queries::lab::MANAGEMENT_NETWORKS_V6,
        "management_network_v6",
    )
    .await
}

/// Get all IPv6 loopback networks currently allocated to labs.
///
/// Returns a vector of `Ipv6Net` representing the IPv6 loopback subnets
/// in use by existing labs. Labs without IPv6 are skipped.
///
/// # Errors
///
/// Fails when the labs cannot be read, or a lab holds a network that does not parse.
#[instrument(skip(db), level = "debug")]
pub async fn get_used_ipv6_loopback_networks(db: &Database) -> Result<Vec<Ipv6Net>> {
    used_networks(
        db,
        queries::lab::LOOPBACK_NETWORKS_V6,
        "loopback_network_v6",
    )
    .await
}

/// The networks labs hold in `column`, read with `query`, each parsed as a `T`.
async fn used_networks<T>(db: &Database, query: Query<0>, column: &str) -> Result<Vec<T>>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let conn = db.reader().await?;
    let rows = query_all(&conn, query.bind([]), lab_network_from_row)
        .await
        .with_context(|| format!("Failed to query labs for their {column}"))?;
    rows.into_iter()
        .map(|(lab_id, network)| {
            network
                .parse()
                .with_context(|| format!("Failed to parse {column} '{network}' for lab '{lab_id}'"))
        })
        .collect()
}

/// A lab's `lab_id` and one network column, from `SELECT lab_id, <network> FROM lab`.
fn lab_network_from_row(row: &Row) -> Result<(String, String)> {
    let mut cells = Cells::new(row);
    Ok((cells.text("lab_id")?, cells.text("network")?))
}
