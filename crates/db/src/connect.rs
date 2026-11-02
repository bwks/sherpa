use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::{Mutex, OwnedMutexGuard};
use tracing::instrument;
use turso::{Builder, Connection};

use shared::konst::{SHERPA_DB_BUSY_TIMEOUT_SECS, SHERPA_DB_FILE_MODE, SHERPA_DB_MEMORY};

use crate::sql::sql_text;

/// Shared handle to Sherpa's embedded database.
///
/// Every operation takes its own connection, because a Turso connection cannot be used by two
/// tasks at once. Writes also hold the handle's write lock, which queues writers in arrival
/// order: under contention, writers left to Turso's busy wait can wait past the busy timeout and
/// fail.
#[derive(Clone)]
pub struct Database {
    inner: turso::Database,
    writes: Arc<Mutex<()>>,
}

/// A connection that holds the database's write lock until it is dropped.
pub(crate) struct Writer {
    pub(crate) conn: Connection,
    _lock: OwnedMutexGuard<()>,
}

impl Database {
    /// A connection for reads. Reads take no lock.
    #[instrument(skip(self), level = "debug")]
    pub(crate) async fn reader(&self) -> Result<Connection> {
        self.connection().await
    }

    /// A connection for writes, holding the write lock.
    #[instrument(skip(self), level = "debug")]
    pub(crate) async fn writer(&self) -> Result<Writer> {
        let lock = Arc::clone(&self.writes).lock_owned().await;
        let conn = self.connection().await?;
        Ok(Writer { conn, _lock: lock })
    }

    async fn connection(&self) -> Result<Connection> {
        let conn = self
            .inner
            .connect()
            .context("Failed to open a database connection")?;
        conn.busy_timeout(Duration::from_secs(SHERPA_DB_BUSY_TIMEOUT_SECS))
            .context("Failed to set the database busy timeout")?;
        conn.execute("PRAGMA foreign_keys = ON", ())
            .await
            .context("Failed to enable database foreign keys")?;
        Ok(conn)
    }

    /// Copy the database to `destination`, a file that must not exist yet, while it stays in use.
    ///
    /// The copy holds the write lock, so it is consistent and writes wait until it finishes.
    #[instrument(skip(self))]
    pub async fn backup(&self, destination: &Path) -> Result<()> {
        let target = destination
            .to_str()
            .context("Backup path is not valid UTF-8")?;
        let writer = self.writer().await?;
        // Turso takes the target only as a string literal, not as a bound parameter.
        writer
            .conn
            .execute(&format!("VACUUM INTO {}", sql_text(target)), ())
            .await
            .context(format!(
                "Failed to back up the database to {}",
                destination.display()
            ))?;
        remove_empty_sidecars(destination)?;
        restrict_permissions(destination)?;
        Ok(())
    }
}

/// Remove the empty WAL and shared-memory files `VACUUM INTO` leaves beside a finished copy, so a
/// backup is a single file.
fn remove_empty_sidecars(path: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm"] {
        let file = format!("{}{suffix}", path.display());
        match fs::metadata(&file) {
            Ok(metadata) if metadata.len() == 0 => {
                fs::remove_file(&file).context(format!("Failed to remove {file}"))?
            }
            Ok(metadata) => {
                tracing::warn!(file = %file, size = metadata.len(), "Backup left a non-empty file; kept");
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(file = %file, "No backup sidecar file");
            }
            Err(error) => {
                return Err(error).context(format!("Failed to read the metadata of {file}"));
            }
        }
    }
    Ok(())
}

/// Limit the database file, and its WAL and shared-memory files when present, to the owner.
fn restrict_permissions(path: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let file = format!("{}{suffix}", path.display());
        match fs::metadata(&file) {
            Ok(_) => fs::set_permissions(&file, fs::Permissions::from_mode(SHERPA_DB_FILE_MODE))
                .context(format!("Failed to restrict the permissions of {file}"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(file = %file, "No database file to restrict");
            }
            Err(error) => {
                return Err(error).context(format!("Failed to read the metadata of {file}"));
            }
        }
    }
    Ok(())
}

/// Open the database file at `path`, creating it if it does not exist.
#[instrument(level = "debug")]
pub async fn open(path: &Path) -> Result<Database> {
    let location = path.to_str().context("Database path is not valid UTF-8")?;
    let inner = Builder::new_local(location)
        .build()
        .await
        .context(format!("Failed to open database {}", path.display()))?;
    if path != Path::new(SHERPA_DB_MEMORY) {
        restrict_permissions(path)?;
    }
    Ok(Database {
        inner,
        writes: Arc::new(Mutex::new(())),
    })
}

/// Open a database that lives only in memory and disappears with the last handle. For tests.
#[instrument(level = "debug")]
pub async fn open_in_memory() -> Result<Database> {
    open(Path::new(SHERPA_DB_MEMORY)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{apply_schema, create_user, find_user};

    #[tokio::test]
    async fn backup_copies_the_database_while_it_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("sherpa.sqlite")).await.unwrap();
        apply_schema(&db).await.unwrap();
        create_user(&db, "alice".into(), "AlicePass123!", false, vec![])
            .await
            .unwrap();

        let copy_path = dir.path().join("backup.sqlite");
        db.backup(&copy_path).await.unwrap();

        let mode = fs::metadata(&copy_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, SHERPA_DB_FILE_MODE);
        assert!(!dir.path().join("backup.sqlite-wal").exists());

        let copy = open(&copy_path).await.unwrap();
        assert!(find_user(&copy, "alice").await.unwrap().is_some());
        // The original stays usable after the backup.
        assert!(find_user(&db, "alice").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn open_limits_the_database_files_to_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sherpa.sqlite");
        fs::write(&path, b"").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let db = open(&path).await.unwrap();
        apply_schema(&db).await.unwrap();
        drop(db);
        let db = open(&path).await.unwrap();
        drop(db);

        for suffix in ["", "-wal"] {
            let file = format!("{}{suffix}", path.display());
            let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, SHERPA_DB_FILE_MODE, "{file}");
        }
    }

    #[tokio::test]
    async fn backup_refuses_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("sherpa.sqlite")).await.unwrap();
        apply_schema(&db).await.unwrap();
        let copy_path = dir.path().join("backup.sqlite");
        std::fs::write(&copy_path, b"keep me").unwrap();

        assert!(db.backup(&copy_path).await.is_err());
        assert_eq!(std::fs::read(&copy_path).unwrap(), b"keep me");
    }

    #[tokio::test]
    async fn writes_from_many_tasks_all_succeed() {
        let db = open_in_memory().await.unwrap();
        apply_schema(&db).await.unwrap();
        let mut tasks = Vec::new();
        for index in 0..32 {
            let db = db.clone();
            tasks.push(tokio::spawn(async move {
                create_user(&db, format!("user{index}"), "UserPass123!", false, vec![]).await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(crate::count_users(&db).await.unwrap(), 32);
    }
}
