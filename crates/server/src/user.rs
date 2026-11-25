//! `sherpad user`: account commands run on the server host, against the database of a stopped
//! server.

use std::path::Path;

use anyhow::{Context, Result, bail};
use tracing::instrument;

use shared::util::normalize_username;

use crate::daemon::pidfile::running_server_pid;
use crate::services::error::ServiceError;
use crate::validate::validate_service_account;

/// The subcommand named when `user unlock` runs as the wrong account.
const UNLOCK_COMMAND: &str = "user unlock";

/// Unlock the account `username`, `sherpa-admin` included, in the database at `db_path`, and set
/// its failed login count back to 0, with `db::unlock_user` as the admin unlock does. `account` is
/// the host account this runs as, which must be `sherpa`. The server holds the database file
/// while it runs, so this refuses, changing nothing, while the PID file at `pid_path` names a
/// running `sherpad`, as `sherpad status` reads it. An account that is not locked is not an
/// error, as for the admin unlock.
///
/// # Errors
///
/// Fails, changing nothing, when it runs as any account but `sherpa`, the server is running,
/// there is no database at `db_path`, or `username` is not a user. Fails when the PID file or
/// the database cannot be read or written.
#[instrument(skip(db_path, pid_path), fields(%username))]
pub async fn unlock(account: &str, username: &str, db_path: &Path, pid_path: &str) -> Result<()> {
    validate_service_account(account, UNLOCK_COMMAND)?;
    if let Some(pid) = running_server_pid(pid_path)? {
        bail!(ServiceError::ServerHoldsDatabase { pid });
    }
    if !db_path.exists() {
        bail!("No database at {}. Nothing was changed", db_path.display());
    }
    let username = normalize_username(username);
    let db = db::open(db_path)
        .await
        .context("Failed to open the database")?;
    let was_locked = db::list_locked_usernames(&db).await?.contains(&username);
    if !db::unlock_user(&db, &username).await? {
        bail!(ServiceError::UnlockUserNotFound { username });
    }
    tracing::info!(
        event = "user.unlocked",
        unlocked_user = %username,
        "Account unlocked"
    );
    if was_locked {
        println!("Unlocked {username}");
    } else {
        println!("{username} was not locked; its failed login count is set back to 0");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;
    use crate::daemon::pidfile::tests::spawn_sherpad_stand_in;

    const LIMIT: u32 = 3;

    async fn stopped_server_with_locked_accounts() -> (TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("sherpa.sqlite");
        let db = db::open(&db_path).await.unwrap();
        db::apply_schema(&db).await.unwrap();
        db::seed_admin_user(&db, "AdminPass123!").await.unwrap();
        db::create_user(&db, "bob".to_owned(), "BobPass123!", false, vec![])
            .await
            .unwrap();
        for username in ["bob", "sherpa-admin"] {
            for _ in 0..LIMIT {
                db::claim_login(&db, username, LIMIT)
                    .await
                    .unwrap()
                    .unwrap();
            }
        }
        assert_eq!(
            db::list_locked_usernames(&db).await.unwrap(),
            ["bob", "sherpa-admin"]
        );
        drop(db);
        let pid_path = dir.path().join("sherpad.pid").to_str().unwrap().to_owned();
        (dir, db_path, pid_path)
    }

    async fn locked_usernames(db_path: &Path) -> Vec<String> {
        let db = db::open(db_path).await.unwrap();
        db::list_locked_usernames(&db).await.unwrap()
    }

    #[tokio::test]
    async fn user_login_029_sherpad_user_unlock_unlocks_an_account_sherpa_admin_included_while_the_server_is_stopped()
     {
        let (_dir, db_path, pid_path) = stopped_server_with_locked_accounts().await;

        unlock("sherpa", "sherpa-admin", &db_path, &pid_path)
            .await
            .unwrap();
        assert_eq!(locked_usernames(&db_path).await, ["bob"]);
        unlock("sherpa", "bob", &db_path, &pid_path).await.unwrap();
        assert_eq!(locked_usernames(&db_path).await, Vec::<String>::new());

        let db = db::open(&db_path).await.unwrap();
        for username in ["bob", "sherpa-admin"] {
            assert!(
                db::claim_login(&db, username, LIMIT)
                    .await
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[tokio::test]
    async fn user_login_030_sherpad_user_unlock_is_refused_while_the_server_runs_and_changes_nothing()
     {
        let (_dir, db_path, pid_path) = stopped_server_with_locked_accounts().await;
        let mut server = spawn_sherpad_stand_in();
        fs::write(&pid_path, server.id().to_string()).unwrap();
        let before = fs::read(&db_path).unwrap();

        let error = unlock("sherpa", "sherpa-admin", &db_path, &pid_path)
            .await
            .unwrap_err();
        let pid = server.id();
        server.kill().unwrap();
        server.wait().unwrap();

        assert!(
            matches!(
                error.downcast_ref::<ServiceError>(),
                Some(ServiceError::ServerHoldsDatabase { pid: running })
                    if u32::try_from(running.as_raw()) == Ok(pid)
            ),
            "{error}"
        );
        assert_eq!(fs::read(&db_path).unwrap(), before);
        assert_eq!(locked_usernames(&db_path).await, ["bob", "sherpa-admin"]);
    }

    #[tokio::test]
    async fn user_login_031_sherpad_user_unlock_of_an_unknown_username_fails_and_changes_nothing() {
        let (_dir, db_path, pid_path) = stopped_server_with_locked_accounts().await;

        let error = unlock("sherpa", "nobody", &db_path, &pid_path)
            .await
            .unwrap_err();

        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::UnlockUserNotFound {
                username: "nobody".to_owned()
            })
        );
        assert_eq!(locked_usernames(&db_path).await, ["bob", "sherpa-admin"]);
    }

    #[tokio::test]
    async fn user_login_032_sherpad_user_unlock_as_another_account_is_refused_with_the_command_to_run()
     {
        let (_dir, db_path, pid_path) = stopped_server_with_locked_accounts().await;
        let before = fs::read(&db_path).unwrap();

        for account in ["root", "bradmin"] {
            let error = unlock(account, "sherpa-admin", &db_path, &pid_path)
                .await
                .unwrap_err();
            assert_eq!(
                error.downcast_ref::<ServiceError>(),
                Some(&ServiceError::NotServiceAccount {
                    command: UNLOCK_COMMAND.to_owned(),
                    account: account.to_owned(),
                })
            );
        }
        assert_eq!(fs::read(&db_path).unwrap(), before);
        assert_eq!(locked_usernames(&db_path).await, ["bob", "sherpa-admin"]);
    }
}
