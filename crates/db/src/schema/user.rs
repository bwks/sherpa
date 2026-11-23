//! User account table schema
//!
//! - `username` is unique and follows the guest-safe username rules: 3 to 32 characters of
//!   `a-z`, `0-9`, `_` and `-`, starting with a letter or `_`.
//! - A user who owns labs cannot be deleted (`lab.user` is `ON DELETE RESTRICT`).
//! - Triggers protect the built-in `sherpa-admin` account
//!   (`sql/migrations/0041_rename_builtin_admin.sql`): it cannot be deleted, renamed or lose its
//!   administrator flag, and no `sherpa-admin` row can exist without the flag.

use anyhow::{Result, bail};
use shared::konst::ADMIN_USERNAME;
use tracing::instrument;
use turso::Connection;

use crate::queries;
use crate::sql::query_count;

pub(crate) const TABLES: &str = include_str!("../../sql/schema/user.sql");

/// Refuse to rename the built-in `admin` account while another user already has its new name.
#[instrument(skip(conn), level = "debug")]
pub(crate) async fn refuse_builtin_admin_name_taken(conn: &Connection) -> Result<()> {
    let taken = query_count(
        conn,
        queries::user::COUNT_BY_USERNAME.bind([ADMIN_USERNAME.into()]),
    )
    .await?;
    if taken > 0 {
        bail!(
            "This release renames the built-in 'admin' account to '{ADMIN_USERNAME}', but a user \
             named '{ADMIN_USERNAME}' already exists. Delete that user with the Sherpa release \
             this server ran before, destroying its labs first, then start this release again. \
             The database is unchanged."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use shared::konst::{ADMIN_PROTECTED_MESSAGE, ADMIN_USERNAME};

    use crate::sql::{Query, query_count};
    use crate::{
        Database, apply_schema, create_admin_user, create_user, delete_user, get_user, list_users,
        open_in_memory, update_user,
    };

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
        query_count(&conn, Query::new("SELECT count(*) FROM user").bind([])).await
    }

    #[tokio::test]
    async fn builtin_admin_005_the_database_refuses_to_delete_rename_or_demote_sherpa_admin()
    -> Result<()> {
        let db = setup().await?;
        let admin = create_admin_user(&db, "AdminPass123!").await?;
        create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;

        let error = execute_raw(&db, "DELETE FROM user").await.unwrap_err();
        assert!(
            error.to_string().contains(ADMIN_PROTECTED_MESSAGE),
            "{error}"
        );
        assert_eq!(user_count(&db).await?, 2, "No user may be deleted");

        for statement in [
            "UPDATE user SET is_admin = 0 WHERE username = 'sherpa-admin'",
            "UPDATE user SET username = 'former_admin' WHERE username = 'sherpa-admin'",
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
            Query::new(
                "SELECT count(*) FROM user WHERE username = 'sherpa-admin' AND is_admin = 1",
            )
            .bind([]),
        )
        .await?;
        assert_eq!(admins, 1);

        let error = delete_user(&db, admin.id.clone().expect("admin id"))
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains(ADMIN_PROTECTED_MESSAGE),
            "{error:#}"
        );
        assert!(get_user(&db, ADMIN_USERNAME).await.is_ok());

        let mut demoted = admin.clone();
        demoted.is_admin = false;
        assert!(update_user(&db, demoted).await.is_err());
        let mut renamed = admin.clone();
        renamed.username = "former_admin".to_string();
        assert!(update_user(&db, renamed).await.is_err());
        let stored = get_user(&db, ADMIN_USERNAME).await?;
        assert!(stored.is_admin);

        let db = setup().await?;
        let error = insert_raw(&db, "sherpa-admin", false).await.unwrap_err();
        assert!(
            error.to_string().contains(ADMIN_PROTECTED_MESSAGE),
            "{error}"
        );
        assert_eq!(user_count(&db).await?, 0);

        insert_raw(&db, "sherpa-admin", true).await?;
        assert_eq!(user_count(&db).await?, 1);

        let db = setup().await?;
        create_admin_user(&db, "AdminPass123!").await?;
        let mut dana = create_user(&db, "dana".to_string(), "TestPass123!", true, vec![]).await?;
        dana.is_admin = false;
        let demoted = update_user(&db, dana).await?;
        assert!(!demoted.is_admin);
        delete_user(&db, demoted.id.clone().unwrap()).await?;
        assert_eq!(list_users(&db).await?.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn user_accounts_007_the_database_refuses_a_username_that_breaks_the_rule_or_is_taken()
    -> Result<()> {
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
