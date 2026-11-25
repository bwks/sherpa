//! Whether this release can use the server's database, checked before it is installed.

use std::path::Path;

use anyhow::{Context, Result};
use tracing::instrument;

/// Check, without changing it, that this release can bring the database at `path` to its
/// schema version, as it does when it starts. A server with no database yet passes, and none
/// is created.
///
/// # Errors
///
/// Fails when the database cannot be opened or read, or this release would refuse it: a newer
/// schema version, or labs that still use the server's shared SSH key, which are named.
#[instrument]
pub async fn check_database(path: &Path) -> Result<()> {
    if !path.exists() {
        println!("No database at {}: nothing to check", path.display());
        return Ok(());
    }
    let db = db::open(path)
        .await
        .context("Failed to open the database")?;
    db::check_schema_upgrade(&db).await?;
    println!("This release can use the database at {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[tokio::test]
    async fn schema_migration_008_check_database_on_a_server_with_no_database_passes_and_creates_none()
     {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sherpa.sqlite");

        check_database(&path).await.unwrap();

        assert!(!path.exists());
    }

    #[tokio::test]
    async fn schema_migration_009_check_database_on_a_database_at_this_release_passes_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sherpa.sqlite");
        {
            let db = db::open(&path).await.unwrap();
            db::apply_schema(&db).await.unwrap();
        }

        let before = fs::read(&path).unwrap();

        check_database(&path).await.unwrap();

        assert_eq!(fs::read(&path).unwrap(), before);
    }
}
