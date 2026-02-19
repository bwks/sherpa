use std::sync::Arc;
use anyhow::{Context, Result, anyhow};
use shared::data::{DbLab, RecordId};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

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
/// # Example
/// ```no_run
/// # use db::{connect, get_lab};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let lab = get_lab(&db, "lab-0001").await?;
/// println!("Found lab: {}", lab.name);
/// # Ok(())
/// # }
/// ```
pub async fn get_lab(db: &Arc<Surreal<Client>>, lab_id: &str) -> Result<DbLab> {
    let mut response = db
        .query("SELECT * FROM ONLY lab WHERE lab_id = $lab_id")
        .bind(("lab_id", lab_id.to_string()))
        .await
        .context(format!("Failed to query lab from database: {}", lab_id))?;

    let db_lab: Option<DbLab> = response.take(0)?;
    db_lab.ok_or_else(|| anyhow!("Lab with lab_id not found: {}", lab_id))
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
/// # Example
/// ```no_run
/// # use db::{connect, get_lab_by_id};
/// # use shared::data::RecordId;
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let id: RecordId = ("lab", "abc123").into();
/// let lab = get_lab_by_id(&db, id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn get_lab_by_id(db: &Arc<Surreal<Client>>, id: RecordId) -> Result<DbLab> {
    let lab: Option<DbLab> = db
        .select(id.clone())
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
/// # Example
/// ```no_run
/// # use db::{connect, get_lab_by_name_and_user, create_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let user = create_user(&db, "alice".to_string(), vec![]).await?;
/// let user_id = user.id.unwrap();
/// let lab = get_lab_by_name_and_user(&db, "My Lab", user_id).await?;
/// # Ok(())
/// # }
/// ```
pub async fn get_lab_by_name_and_user(
    db: &Arc<Surreal<Client>>,
    name: &str,
    user_id: RecordId,
) -> Result<DbLab> {
    let mut response = db
        .query("SELECT * FROM ONLY lab WHERE name = $name AND user = $user_id")
        .bind(("name", name.to_string()))
        .bind(("user_id", user_id.clone()))
        .await
        .context(format!("Failed to query lab by name and user: {}", name))?;

    let db_lab: Option<DbLab> = response.take(0)?;
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
/// # Example
/// ```no_run
/// # use db::{connect, list_labs};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let labs = list_labs(&db).await?;
/// println!("Found {} labs", labs.len());
/// # Ok(())
/// # }
/// ```
pub async fn list_labs(db: &Arc<Surreal<Client>>) -> Result<Vec<DbLab>> {
    let labs: Vec<DbLab> = db
        .select("lab")
        .await
        .context("Failed to list labs from database")?;

    Ok(labs)
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
/// # Example
/// ```no_run
/// # use db::{connect, list_labs_by_user, create_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let user = create_user(&db, "alice".to_string(), vec![]).await?;
/// let user_id = user.id.unwrap();
/// let labs = list_labs_by_user(&db, user_id).await?;
/// println!("User owns {} labs", labs.len());
/// # Ok(())
/// # }
/// ```
pub async fn list_labs_by_user(db: &Arc<Surreal<Client>>, user_id: RecordId) -> Result<Vec<DbLab>> {
    let mut response = db
        .query("SELECT * FROM lab WHERE user = $user_id")
        .bind(("user_id", user_id.clone()))
        .await
        .context(format!("Failed to list labs for user: {:?}", user_id))?;

    let labs: Vec<DbLab> = response.take(0)?;
    Ok(labs)
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
/// # Example
/// ```no_run
/// # use db::{connect, count_labs};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let count = count_labs(&db).await?;
/// println!("Total labs: {}", count);
/// # Ok(())
/// # }
/// ```
pub async fn count_labs(db: &Arc<Surreal<Client>>) -> Result<usize> {
    let mut response = db
        .query("SELECT count() FROM lab GROUP ALL")
        .await
        .context("Failed to count labs")?;

    let count: Option<usize> = response.take("count")?;
    Ok(count.unwrap_or(0))
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
/// # Example
/// ```no_run
/// # use db::{connect, count_labs_by_user, create_user};
/// # use shared::konst::{SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME};
/// # async fn example() -> anyhow::Result<()> {
/// let db = connect(SHERPA_DB_SERVER, SHERPA_DB_PORT, SHERPA_DB_NAMESPACE, SHERPA_DB_NAME).await?;
/// let user = create_user(&db, "alice".to_string(), vec![]).await?;
/// let user_id = user.id.unwrap();
/// let count = count_labs_by_user(&db, user_id).await?;
/// println!("User owns {} labs", count);
/// # Ok(())
/// # }
/// ```
pub async fn count_labs_by_user(db: &Arc<Surreal<Client>>, user_id: RecordId) -> Result<usize> {
    let mut response = db
        .query("SELECT count() FROM lab WHERE user = $user_id GROUP ALL")
        .bind(("user_id", user_id))
        .await
        .context("Failed to count labs for user")?;

    let count: Option<usize> = response.take("count")?;
    Ok(count.unwrap_or(0))
}
