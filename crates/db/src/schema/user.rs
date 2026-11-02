//! User account table schema
//!
//! - `username` is unique and follows the guest-safe username rules: 3 to 32 characters of
//!   `a-z`, `0-9`, `_` and `-`, starting with a letter or `_`.
//! - A user who owns labs cannot be deleted (`lab.user` is `ON DELETE RESTRICT`).
//! - Triggers protect the built-in `admin` account: it cannot be deleted, renamed or lose its
//!   administrator flag, and no `admin` row can exist without the flag.
use shared::konst::{ADMIN_PROTECTED_MESSAGE, ADMIN_USERNAME};

use crate::sql::sql_text;

pub(crate) const TABLES: &str = "
CREATE TABLE user (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE
        CHECK (length(username) BETWEEN 3 AND 32
            AND substr(username, 1, 1) GLOB '[a-z_]'
            AND username NOT GLOB '*[^a-z0-9_-]*'),
    password_hash TEXT NOT NULL,
    is_admin INTEGER NOT NULL CHECK (is_admin IN (0, 1)),
    ssh_keys TEXT NOT NULL CHECK (json_valid(ssh_keys)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
";

/// The triggers protecting the built-in admin account.
pub(crate) fn rules() -> String {
    let admin = sql_text(ADMIN_USERNAME);
    let message = sql_text(ADMIN_PROTECTED_MESSAGE);
    format!(
        "DROP TRIGGER IF EXISTS user_protect_admin_insert;
CREATE TRIGGER user_protect_admin_insert BEFORE INSERT ON user
    WHEN NEW.username = {admin} AND NEW.is_admin != 1
    BEGIN SELECT RAISE(ABORT, {message}); END;
DROP TRIGGER IF EXISTS user_protect_admin_update;
CREATE TRIGGER user_protect_admin_update BEFORE UPDATE ON user
    WHEN (OLD.username = {admin} AND (NEW.username != {admin} OR NEW.is_admin != 1))
        OR (NEW.username = {admin} AND NEW.is_admin != 1)
    BEGIN SELECT RAISE(ABORT, {message}); END;
DROP TRIGGER IF EXISTS user_protect_admin_delete;
CREATE TRIGGER user_protect_admin_delete BEFORE DELETE ON user
    WHEN OLD.username = {admin}
    BEGIN SELECT RAISE(ABORT, {message}); END;
"
    )
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use shared::konst::ADMIN_PROTECTED_MESSAGE;

    use crate::sql::query_count;
    use crate::{Database, apply_schema, create_admin_user, create_user, open_in_memory};

    async fn setup() -> Result<Database> {
        let db = open_in_memory().await?;
        apply_schema(&db).await?;
        Ok(db)
    }

    /// Insert a user row directly, below the db functions' validation.
    async fn insert_raw(db: &Database, username: &str, is_admin: bool) -> Result<u64> {
        let writer = db.writer().await?;
        Ok(writer
            .conn
            .execute(
                "INSERT INTO user (id, username, password_hash, is_admin, ssh_keys, created_at, \
                 updated_at) VALUES (?1, ?1, 'x', ?2, '[]', '2026-01-01T00:00:00Z', \
                 '2026-01-01T00:00:00Z')",
                (username, is_admin),
            )
            .await?)
    }

    async fn execute_raw(db: &Database, sql: &str) -> Result<u64> {
        let writer = db.writer().await?;
        Ok(writer.conn.execute(sql, ()).await?)
    }

    async fn user_count(db: &Database) -> Result<usize> {
        let conn = db.reader().await?;
        query_count(&conn, "SELECT count(*) FROM user", ()).await
    }

    #[tokio::test]
    async fn bulk_delete_is_refused_while_admin_exists() -> Result<()> {
        let db = setup().await?;
        create_admin_user(&db, "AdminPass123!").await?;
        create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;

        let error = execute_raw(&db, "DELETE FROM user").await.unwrap_err();
        assert!(
            error.to_string().contains(ADMIN_PROTECTED_MESSAGE),
            "{error}"
        );
        assert_eq!(user_count(&db).await?, 2, "No user may be deleted");
        Ok(())
    }

    #[tokio::test]
    async fn raw_updates_cannot_rename_or_demote_admin() -> Result<()> {
        let db = setup().await?;
        create_admin_user(&db, "AdminPass123!").await?;

        for statement in [
            "UPDATE user SET is_admin = 0 WHERE username = 'admin'",
            "UPDATE user SET username = 'former_admin' WHERE username = 'admin'",
        ] {
            let error = execute_raw(&db, statement).await.unwrap_err();
            assert!(
                error.to_string().contains(ADMIN_PROTECTED_MESSAGE),
                "{statement}: {error}"
            );
        }
        let conn = db.reader().await?;
        let admins = query_count(
            &conn,
            "SELECT count(*) FROM user WHERE username = 'admin' AND is_admin = 1",
            (),
        )
        .await?;
        assert_eq!(admins, 1);
        Ok(())
    }

    #[tokio::test]
    async fn admin_row_without_admin_flag_is_refused() -> Result<()> {
        let db = setup().await?;

        let error = insert_raw(&db, "admin", false).await.unwrap_err();
        assert!(
            error.to_string().contains(ADMIN_PROTECTED_MESSAGE),
            "{error}"
        );
        assert_eq!(user_count(&db).await?, 0);

        insert_raw(&db, "admin", true).await?;
        assert_eq!(user_count(&db).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn schema_rejects_username_written_directly() -> Result<()> {
        let db = setup().await?;

        for username in ["Brad", "brad.searle", "ab", "1brad", "brad searle"] {
            assert!(
                insert_raw(&db, username, false).await.is_err(),
                "Schema should reject '{username}'"
            );
        }
        // The same statement succeeds for valid names, so the rejections come from the username.
        for username in ["brad", "_svc", "brad-searle_2"] {
            insert_raw(&db, username, false).await?;
        }
        assert_eq!(user_count(&db).await?, 3);
        Ok(())
    }

    #[tokio::test]
    async fn usernames_are_unique() -> Result<()> {
        let db = setup().await?;
        insert_raw(&db, "brad", false).await?;
        let error = execute_raw(
            &db,
            "INSERT INTO user (id, username, password_hash, is_admin, ssh_keys, created_at, \
             updated_at) VALUES ('other', 'brad', 'x', 0, '[]', '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z')",
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("UNIQUE"), "{error}");
        Ok(())
    }
}
