use anyhow::{Context, Result, anyhow};
use shared::data::{DbLab, DbLink, DbNode, RecordId};
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::helpers::get_lab_id;
use crate::lab::get_lab;

/// Delete a lab by its lab_id (business key)
///
/// **WARNING:** This function only deletes the lab record itself.
/// With CASCADE DELETE enabled in the schema, nodes and links will be
/// automatically deleted by the database.
///
/// For explicit control over deletion order, use `delete_lab_cascade()`.
/// To check for dependencies before deletion, use `delete_lab_safe()`.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Errors
/// - If lab not found
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_lab};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// delete_lab(&db, "lab-0001").await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_lab(db: &Arc<Surreal<Client>>, lab_id: &str) -> Result<()> {
    let lab = get_lab(db, lab_id).await?;
    let lab_record_id = get_lab_id(&lab)?;

    let _deleted: Option<DbLab> = db
        .delete(lab_record_id)
        .await
        .context(format!("Failed to delete lab: {}", lab_id))?;

    Ok(())
}

/// Delete a lab by its RecordId (surrogate key)
///
/// **WARNING:** This function only deletes the lab record itself.
/// With CASCADE DELETE enabled in the schema, nodes and links will be
/// automatically deleted by the database.
///
/// # Arguments
/// * `db` - Database connection
/// * `id` - The RecordId of the lab
///
/// # Errors
/// - If lab not found
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_lab_by_id};
/// # use shared::data::RecordId;
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let id: RecordId = ("lab", "abc123").into();
/// delete_lab_by_id(&db, id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_lab_by_id(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<()> {
    let _deleted: Option<DbLab> = db
        .delete(id.clone())
        .await
        .context(format!("Failed to delete lab by id: {:?}", id))?;

    Ok(())
}

/// Delete all nodes for a lab
///
/// This is a helper function used by `delete_lab_cascade()`.
/// When CASCADE DELETE is enabled in the schema, this may not be necessary
/// as the database will automatically delete nodes when the lab is deleted.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Errors
/// - If lab not found
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_lab_nodes};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// delete_lab_nodes(&db, "lab-0001").await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_lab_nodes(db: &Arc<Surreal<Client>>, lab_id: &str) -> Result<()> {
    let lab = get_lab(db, lab_id).await?;
    let lab_record_id = get_lab_id(&lab)?;

    let _deleted: Vec<DbNode> = db
        .query("DELETE node WHERE lab = $lab_record_id")
        .bind(("lab_record_id", lab_record_id))
        .await
        .context(format!("Failed to delete nodes for lab: {}", lab_id))?
        .take(0)?;

    Ok(())
}

/// Delete all links for a lab
///
/// This is a helper function used by `delete_lab_cascade()`.
/// When CASCADE DELETE is enabled in the schema, this may not be necessary
/// as the database will automatically delete links when the lab is deleted.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Errors
/// - If lab not found
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_lab_links};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// delete_lab_links(&db, "lab-0001").await?;
/// # Ok(())
/// # }
/// ```
pub async fn delete_lab_links(db: &Arc<Surreal<Client>>, lab_id: &str) -> Result<()> {
    let lab = get_lab(db, lab_id).await?;
    let lab_record_id = get_lab_id(&lab)?;

    let _deleted: Vec<DbLink> = db
        .query("DELETE link WHERE lab = $lab_record_id")
        .bind(("lab_record_id", lab_record_id))
        .await
        .context(format!("Failed to delete links for lab: {}", lab_id))?
        .take(0)?;

    Ok(())
}

/// Delete a lab and cascade delete all dependent records.
///
/// Deletes the lab record. The database automatically cascade deletes
/// all associated nodes, links, and bridges via `REFERENCE ON DELETE CASCADE`
/// constraints defined in the schema.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Errors
/// - If lab not found
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_lab_cascade};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// delete_lab_cascade(&db, "lab-0001").await?;
/// println!("Lab and all its dependencies deleted");
/// # Ok(())
/// # }
/// ```
pub async fn delete_lab_cascade(db: &Arc<Surreal<Client>>, lab_id: &str) -> Result<()> {
    delete_lab(db, lab_id).await
}

/// Delete a lab safely (only if it has no nodes or links)
///
/// This function checks if the lab has any nodes or links before deletion.
/// If dependencies exist, it returns an error with details about what's blocking deletion.
///
/// Use this when you want to prevent accidental deletion of labs that still contain resources.
///
/// # Arguments
/// * `db` - Database connection
/// * `lab_id` - The unique lab_id string
///
/// # Errors
/// - If lab not found
/// - If lab has nodes or links (won't delete)
/// - If there's a database error
///
/// # Example
/// ```no_run
/// # use db::{connect, delete_lab_safe};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
///
/// match delete_lab_safe(&db, "lab-0001").await {
///     Ok(_) => println!("Lab deleted successfully"),
///     Err(e) => println!("Cannot delete lab: {}", e),
/// }
/// # Ok(())
/// # }
/// ```
pub async fn delete_lab_safe(db: &Arc<Surreal<Client>>, lab_id: &str) -> Result<()> {
    // Get the lab to verify it exists
    let lab = get_lab(db, lab_id).await?;
    let lab_record_id = get_lab_id(&lab)?;

    // Check for nodes
    let nodes: Vec<DbNode> = db
        .query("SELECT * FROM node WHERE lab = $lab_record_id ORDER BY name ASC")
        .bind(("lab_record_id", lab_record_id.clone()))
        .await
        .context("Failed to check for nodes")?
        .take(0)?;

    if !nodes.is_empty() {
        return Err(anyhow!(
            "Cannot delete lab '{}' ({}): lab contains {} node(s). Delete nodes first or use delete_lab_cascade()",
            lab.name,
            lab_id,
            nodes.len()
        ));
    }

    // Check for links
    let links: Vec<DbLink> = db
        .query("SELECT * FROM link WHERE lab = $lab_record_id")
        .bind(("lab_record_id", lab_record_id.clone()))
        .await
        .context("Failed to check for links")?
        .take(0)?;

    if !links.is_empty() {
        return Err(anyhow!(
            "Cannot delete lab '{}' ({}): lab contains {} link(s). Delete links first or use delete_lab_cascade()",
            lab.name,
            lab_id,
            links.len()
        ));
    }

    // Safe to delete - lab is empty
    delete_lab(db, lab_id).await
}
