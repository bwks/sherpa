//! Schema application and orchestration
//!
//! This module handles the application of all database schemas to SurrealDB.
//! It imports schema definitions from individual modules and applies them
//! in the correct dependency order to ensure foreign key relationships
//! are properly established.

use anyhow::{Context, Result};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use super::bridge::generate_bridge_schema;
use super::lab::generate_lab_schema;
use super::link::generate_link_schema;
use super::node::generate_node_schema;
use super::node_config::generate_node_config_schema;
use super::user::generate_user_schema;

/// Apply a single schema section to the database.
///
/// Executes the provided schema SQL against the database and provides
/// contextual error messages if the operation fails.
///
/// # Parameters
///
/// * `db` - The SurrealDB client connection
/// * `section_name` - Human-readable name of the schema section (for logging)
/// * `schema` - The SQL schema definition to execute
///
/// # Returns
///
/// * `Ok(())` if the schema was applied successfully
/// * `Err` if the schema application failed
///
/// # Examples
///
/// ```ignore
/// let user_schema = generate_user_schema();
/// apply_schema_section(&db, "user", &user_schema).await?;
/// ```
async fn apply_schema_section(
    db: &Surreal<Client>,
    section_name: &str,
    schema: &str,
) -> Result<()> {
    println!("Creating table: {}", section_name);

    db.query(schema)
        .await
        .context(format!("Failed to apply schema: {}", section_name))?;

    Ok(())
}

/// Apply all database schemas in the correct dependency order.
///
/// This function creates all tables, fields, indexes, and constraints
/// required by the Sherpa application. It is idempotent - safe to run
/// multiple times without errors.
///
/// The schema constraints are automatically generated from Rust enums,
/// ensuring type safety and preventing enum variant mismatches between
/// the database schema and application code.
///
/// # Order of Execution
///
/// Tables are created in dependency order to satisfy foreign key relationships:
/// 1. **user** (no dependencies)
/// 2. **node_config** (no dependencies)
/// 3. **lab** (depends on: user)
/// 4. **node** (depends on: node_config, lab)
/// 5. **link** (depends on: node, lab)
///
/// # Parameters
///
/// * `db` - The SurrealDB client connection
///
/// # Returns
///
/// * `Ok(())` if all schemas were applied successfully
/// * `Err` if any schema application failed
///
/// # Error Handling
///
/// If any schema fails to apply, the function returns immediately with
/// an error. Subsequent schemas will not be applied. Since SurrealDB
/// schema operations are idempotent, you can safely retry the operation
/// after fixing any issues.
///
/// # Examples
///
/// ```no_run
/// use db::{connect, apply_schema};
/// use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
///     apply_schema(&db).await?;
///     Ok(())
/// }
/// ```
pub async fn apply_schema(db: &Surreal<Client>) -> Result<()> {
    // Generate schemas dynamically from individual schema modules
    let user_schema = generate_user_schema();
    let node_config_schema = generate_node_config_schema();
    let lab_schema = generate_lab_schema();
    let node_schema = generate_node_schema();
    let link_schema = generate_link_schema();
    let bridge_schema = generate_bridge_schema();

    // Apply schemas in dependency order
    apply_schema_section(db, "user", &user_schema).await?;
    apply_schema_section(db, "node_config", &node_config_schema).await?;
    apply_schema_section(db, "lab", &lab_schema).await?;
    apply_schema_section(db, "node", &node_schema).await?;
    apply_schema_section(db, "link", &link_schema).await?;
    apply_schema_section(db, "bridge", &bridge_schema).await?;

    Ok(())
}
