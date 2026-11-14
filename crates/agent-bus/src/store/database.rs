//! Opening the record and bringing it to the current schema.
//!
//! The record is one Turso file, `agent-bus.db` in the host's state directory, with its `-wal`
//! file beside it, all 0600. Turso as `crates/db` uses it holds the file for one process, so
//! only the process holding `record.lock` (`crate::record`) opens it.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tracing::instrument;
use turso::{Builder, Connection, Database};

use crate::files::{PRIVATE_FILE_MODE, private_dir};
use crate::store::sql::MIGRATIONS;

/// The record's file name in the state directory.
pub const FILE: &str = "agent-bus.db";
/// How long a statement waits for another connection's write in this process.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// The location Turso takes for a database in memory.
#[cfg(test)]
const MEMORY: &str = ":memory:";

/// The record, open in this process.
#[derive(Clone)]
pub struct Store {
    db: Database,
}

impl Store {
    /// Open the record at `path`, creating it (and its directory, 0700) if missing, restrict it
    /// to the owner, and bring it to the current schema.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened (another process holds it), its mode set, or its
    /// schema is newer than this agent-bus knows.
    #[instrument(skip_all, fields(path = %path.display()))]
    pub async fn open(path: &Path) -> Result<Self> {
        let location = path
            .to_str()
            .context("The record's path is not valid UTF-8")?;
        if let Some(dir) = path.parent() {
            private_dir(dir)?;
        }
        let db = Builder::new_local(location)
            .build()
            .await
            .with_context(|| format!("Unable to open the record {}", path.display()))?;
        let store = Self { db };
        store.migrate().await?;
        restrict(path)?;
        Ok(store)
    }

    /// A record in memory, at the current schema, for tests.
    ///
    /// # Errors
    ///
    /// Fails when the schema cannot be applied.
    #[cfg(test)]
    pub async fn open_in_memory() -> Result<Self> {
        let db = Builder::new_local(MEMORY).build().await?;
        let store = Self { db };
        store.migrate().await?;
        Ok(store)
    }

    /// A new connection. Turso's connections are not shared between tasks.
    ///
    /// # Errors
    ///
    /// Fails when Turso refuses a connection.
    pub fn connect(&self) -> Result<Connection> {
        let conn = self
            .db
            .connect()
            .context("Unable to open a connection to the record")?;
        conn.busy_timeout(BUSY_TIMEOUT)
            .context("Unable to set the record's busy timeout")?;
        Ok(conn)
    }

    /// Bring the record to the current schema version, in one transaction.
    #[instrument(level = "debug", skip_all)]
    async fn migrate(&self) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn
            .transaction()
            .await
            .context("Unable to start the schema transaction")?;
        let version = schema_version(&tx).await?;
        let current = MIGRATIONS.len();
        if version > current {
            bail!(
                "The record has schema version {version}, newer than {current} that this \
                 agent-bus knows: use a newer agent-bus"
            );
        }
        for (target, migration) in MIGRATIONS.iter().enumerate().skip(version) {
            let target = target + 1;
            tx.execute_batch(migration)
                .await
                .with_context(|| format!("Unable to migrate the record to version {target}"))?;
            tx.execute(&format!("PRAGMA user_version = {target}"), ())
                .await
                .context("Unable to record the schema version")?;
        }
        tx.commit()
            .await
            .context("Unable to commit the schema transaction")
    }
}

/// The schema version the record says it has.
async fn schema_version(conn: &Connection) -> Result<usize> {
    let mut rows = conn
        .query("PRAGMA user_version", ())
        .await
        .context("Unable to read the schema version")?;
    let row = rows
        .next()
        .await
        .context("Unable to read the schema version")?
        .context("No schema version")?;
    let version = row
        .get_value(0)
        .context("No schema version")?
        .as_integer()
        .copied()
        .context("The schema version is not a number")?;
    usize::try_from(version).context("The schema version is negative")
}

/// Restrict the record and its `-wal` and `-shm` files, those that exist, to the owner.
fn restrict(path: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        match fs::set_permissions(&file, fs::Permissions::from_mode(PRIVATE_FILE_MODE)) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Unable to set the mode of {}", Path::new(&file).display())
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;
    use std::{env, process};

    use turso::Value;

    use super::*;
    use crate::store::sql::QUERIES;

    #[tokio::test]
    async fn every_query_fits_the_migrated_schema() {
        let store = Store::open_in_memory().await.unwrap();
        let conn = store.connect().unwrap();
        for (name, sql) in QUERIES {
            if let Err(error) = conn.prepare(sql).await {
                panic!("sql/queries/{name} does not fit the schema: {error}");
            }
        }
        assert_eq!(schema_version(&conn).await.unwrap(), MIGRATIONS.len());
    }

    #[test]
    fn every_query_file_is_checked() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("sql/queries");
        let mut files: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort_unstable();
        let mut listed: Vec<String> = QUERIES.iter().map(|(name, _)| (*name).to_owned()).collect();
        listed.sort_unstable();
        assert_eq!(files, listed);
    }

    #[test]
    fn every_migration_file_is_applied_in_order() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("sql/migrations");
        let mut files: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort_unstable();
        assert_eq!(files.len(), MIGRATIONS.len());
        for (index, file) in files.iter().enumerate() {
            assert!(
                file.starts_with(&format!("{:04}_", index + 1)),
                "{file} is out of sequence"
            );
        }
    }

    #[tokio::test]
    async fn a_record_file_is_private_and_opens_again_at_the_same_version() {
        let dir = env::temp_dir().join(format!("agent-bus-store-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        let store = Store::open(&path).await.unwrap();
        drop(store);
        assert_eq!(
            fs::metadata(&path).unwrap().mode() & 0o777,
            PRIVATE_FILE_MODE
        );
        let store = Store::open(&path).await.unwrap();
        let conn = store.connect().unwrap();
        assert_eq!(schema_version(&conn).await.unwrap(), MIGRATIONS.len());
        drop(conn);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_record_newer_than_this_agent_bus_is_refused() {
        let dir = env::temp_dir().join(format!("agent-bus-store-newer-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        let store = Store::open(&path).await.unwrap();
        store
            .connect()
            .unwrap()
            .execute(
                &format!("PRAGMA user_version = {}", MIGRATIONS.len() + 1),
                (),
            )
            .await
            .unwrap();
        drop(store);
        let error = Store::open(&path).await.err().unwrap();
        assert!(format!("{error:#}").contains("newer"), "{error:#}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_version_1_record_with_rows_migrates_and_keeps_them() {
        // Every later migration runs over it: the run's time columns are added empty.
        let dir = env::temp_dir().join(format!("agent-bus-store-v1-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        let location = path.to_str().unwrap();
        {
            // A record as agent-bus wrote it at schema version 1.
            let db = Builder::new_local(location).build().await.unwrap();
            let conn = db.connect().unwrap();
            conn.execute_batch(MIGRATIONS[0]).await.unwrap();
            conn.execute("PRAGMA user_version = 1", ()).await.unwrap();
            conn.execute_batch(
                "INSERT INTO message (seq, at_ms, task, from_agent, to_agent, kind, text, model, \
                     github, usage, task_usage) \
                 VALUES (1, 1000, 't1', 'host', 'dev', 'task', 'do it', NULL, 0, NULL, NULL); \
                 INSERT INTO task (id, agent, requester, first_seq, last_seq, last_at_ms, github, \
                     state, questions, answers, runs, cost_usd) \
                 VALUES ('t1', 'dev', 'host', 1, 1, 1000, 0, 'completed', 0, 0, 1, 0.5);",
            )
            .await
            .unwrap();
        }
        let store = Store::open(&path).await.unwrap();
        let conn = store.connect().unwrap();
        assert_eq!(schema_version(&conn).await.unwrap(), MIGRATIONS.len());
        let mut rows = conn
            .query("SELECT text, work, ended FROM message WHERE seq = 1", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get_value(0).unwrap(), Value::Text("do it".to_owned()));
        assert_eq!(row.get_value(1).unwrap(), Value::Null);
        assert_eq!(row.get_value(2).unwrap(), Value::Null);
        drop(rows);
        let mut rows = conn
            .query("SELECT state, cost_usd FROM task WHERE id = 't1'", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(
            row.get_value(0).unwrap(),
            Value::Text("completed".to_owned())
        );
        assert_eq!(row.get_value(1).unwrap(), Value::Real(0.5));
        drop(rows);
        // The new tables are there, empty.
        for table in [
            "work_item",
            "work_ref",
            "task_ref",
            "task_work",
            "host_transcript",
            "host_usage",
            "host_span",
            "host_focus",
            "host_meta",
        ] {
            let mut rows = conn
                .query(&format!("SELECT count(*) FROM {table}"), ())
                .await
                .unwrap();
            let row = rows.next().await.unwrap().unwrap();
            assert_eq!(row.get_value(0).unwrap(), Value::Integer(0), "{table}");
        }
        drop(conn);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }
}
