//! Checking a username and password, for every login route.

use std::sync::LazyLock;

use anyhow::{Context, Result};
use db::Database;
use shared::auth::password;
use shared::data::DbUser;
use tracing::instrument;

/// Checked in place of a real hash when the username does not exist.
const DUMMY_PASSWORD: &str = "Unknown-User-Password-1!";

/// A hash made with the same parameters as real password hashes, so checking against it takes
/// as long as checking a real one.
static DUMMY_HASH: LazyLock<Option<String>> =
    LazyLock::new(|| password::hash_password(DUMMY_PASSWORD).ok());

/// The user `username`, if `password` is theirs. `None` means the username is unknown or the
/// password is wrong; callers report both the same way.
///
/// An unknown username costs the same as a wrong password: the password is still checked,
/// against a dummy hash, so response times do not show which usernames exist.
///
/// # Errors
///
/// Fails when the database cannot be read or a stored hash cannot be checked.
#[instrument(skip(db, password), fields(%username))]
pub async fn check_credentials(
    db: &Database,
    username: &str,
    password: &str,
) -> Result<Option<DbUser>> {
    let user = db::find_user(db, username)
        .await
        .context("Failed to look up user")?;
    let Some(user) = user else {
        match DUMMY_HASH.as_deref() {
            Some(hash) => {
                // The result is ignored: the user does not exist either way.
                let _ = password::verify_password(password, hash);
            }
            None => tracing::error!("No dummy password hash; unknown usernames answer faster"),
        }
        tracing::debug!("Login attempt for an unknown user");
        return Ok(None);
    };
    let matches = password::verify_password(password, &user.password_hash)
        .context("Failed to check the password")?;
    if !matches {
        tracing::debug!("Login attempt with a wrong password");
        return Ok(None);
    }
    Ok(Some(user))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "Original-Pass-123!";

    async fn database_with_alice() -> Database {
        let db = db::open_in_memory().await.expect("open database");
        db::apply_schema(&db).await.expect("apply schema");
        db::create_user(&db, "alice".to_owned(), PASSWORD, false, vec![])
            .await
            .expect("create user");
        db
    }

    #[tokio::test]
    async fn the_right_password_returns_the_user() {
        let db = database_with_alice().await;
        let user = check_credentials(&db, "Alice", PASSWORD)
            .await
            .expect("check");
        assert_eq!(user.map(|user| user.username), Some("alice".to_owned()));
    }

    #[tokio::test]
    async fn a_wrong_password_and_an_unknown_user_look_the_same() {
        let db = database_with_alice().await;
        assert!(
            check_credentials(&db, "alice", "Wrong-Pass-789!")
                .await
                .expect("check")
                .is_none()
        );
        assert!(
            check_credentials(&db, "nobody", PASSWORD)
                .await
                .expect("check")
                .is_none()
        );
    }

    #[test]
    fn the_dummy_hash_is_a_real_hash() {
        let hash = DUMMY_HASH.as_deref().expect("dummy hash");
        assert!(password::verify_password(DUMMY_PASSWORD, hash).expect("verify"));
    }
}
