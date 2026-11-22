use anyhow::{Context, Result};
use shared::util::normalize_username;
use tracing::instrument;

use crate::Database;
use crate::persistence::Cells;
use crate::queries;
use crate::sql::{query_all, query_optional};

/// Unlock the account `username` and set its failed login count back to 0. Returns whether the
/// user exists; unlocking an account that is not locked is not an error.
///
/// # Errors
/// - If there's a database error during the update
#[instrument(skip(db), level = "debug")]
pub async fn unlock_user(db: &Database, username: &str) -> Result<bool> {
    let writer = db.writer().await?;
    let unlocked = query_optional(
        &writer.conn,
        queries::user::UNLOCK.bind([normalize_username(username).into()]),
        |_| Ok(()),
    )
    .await
    .context(format!("Failed to unlock user {username}"))?;
    Ok(unlocked.is_some())
}

/// The usernames of the locked accounts, in order.
///
/// # Errors
/// - If there's a database error during the query
#[instrument(skip(db), level = "debug")]
pub async fn list_locked_usernames(db: &Database) -> Result<Vec<String>> {
    let conn = db.reader().await?;
    query_all(&conn, queries::user::LIST_LOCKED.bind([]), |row| {
        Cells::new(row).text("username")
    })
    .await
    .context("Failed to list the locked accounts")
}

#[cfg(test)]
mod tests {
    use turso::Builder;

    use super::*;
    use crate::{apply_schema, claim_login, open, seed_admin_user};

    /// The statement the user docs give root to unlock `admin` with `sqlite3`, while sherpad is
    /// stopped.
    const ROOT_UNLOCK_SQL: &str =
        "UPDATE user SET failed_logins = 0, locked_at = NULL WHERE username = 'admin';";
    const LIMIT: u32 = 3;

    #[tokio::test]
    async fn user_login_029_the_documented_root_sql_unlocks_admin_on_a_stopped_server() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sherpa.sqlite");
        let db = open(&path).await.unwrap();
        apply_schema(&db).await.unwrap();
        seed_admin_user(&db, "AdminPass123!").await.unwrap();
        for _ in 0..LIMIT {
            claim_login(&db, "admin", LIMIT).await.unwrap().unwrap();
        }
        assert_eq!(list_locked_usernames(&db).await.unwrap(), ["admin"]);
        drop(db);

        // A plain connection to the file, as `sqlite3` makes, with nothing of the crate's.
        let file = Builder::new_local(path.to_str().unwrap())
            .build()
            .await
            .unwrap();
        let conn = file.connect().unwrap();
        assert_eq!(conn.execute(ROOT_UNLOCK_SQL, ()).await.unwrap(), 1);
        drop(conn);
        drop(file);

        let db = open(&path).await.unwrap();
        assert_eq!(
            list_locked_usernames(&db).await.unwrap(),
            Vec::<String>::new()
        );
        assert!(claim_login(&db, "admin", LIMIT).await.unwrap().is_some());
    }
}
