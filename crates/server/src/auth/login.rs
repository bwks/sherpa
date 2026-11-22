//! Checking a username and password, for every login route.

use std::net::IpAddr;
use std::sync::LazyLock;

use anyhow::{Context, Result};
use db::Database;
use shared::auth::password;
use shared::data::DbUser;
use tracing::instrument;

use crate::auth::login_limit::LoginLimiter;

/// Checked in place of a real hash when the username does not exist.
const DUMMY_PASSWORD: &str = "Unknown-User-Password-1!";

/// A hash made with the same parameters as real password hashes, so checking against it takes
/// as long as checking a real one.
static DUMMY_HASH: LazyLock<Option<String>> =
    LazyLock::new(|| password::hash_password(DUMMY_PASSWORD).ok());

/// The user `username`, if `password` is theirs and the login limit lets the attempt from
/// `address` through. `None` means the username is unknown, the password is wrong or the limit
/// refused the attempt; callers report all three the same way.
///
/// An unknown username costs the same as a wrong password: the password is still checked,
/// against a dummy hash, so response times do not show which usernames exist. An attempt the
/// limit refuses is checked against the dummy hash too, never against the account's own.
///
/// # Errors
///
/// Fails when the database cannot be read, a stored hash cannot be checked, or the login limit's
/// counts cannot be read or written.
#[instrument(skip(db, limiter, password), fields(%username, %address))]
pub async fn check_credentials(
    db: &Database,
    limiter: &LoginLimiter,
    username: &str,
    password: &str,
    address: IpAddr,
) -> Result<Option<DbUser>> {
    let Some(attempt) = limiter.begin(username, address).await? else {
        check_dummy_hash(password);
        tracing::debug!("Login attempt refused by the login limit");
        return Ok(None);
    };
    let user = match match_password(db, username, password).await {
        Ok(user) => user,
        Err(error) => {
            if let Err(give_back) = attempt.unanswered().await {
                tracing::warn!(
                    error = format!("{give_back:#}"),
                    "Login attempt stays counted"
                );
            }
            return Err(error);
        }
    };
    if user.is_some() {
        attempt.succeeded().await?;
    } else {
        attempt.failed();
    }
    Ok(user)
}

/// The user `username`, if `password` is theirs, with an unknown username costing as much as a
/// wrong password.
async fn match_password(db: &Database, username: &str, password: &str) -> Result<Option<DbUser>> {
    let user = db::find_user(db, username)
        .await
        .context("Failed to look up user")?;
    let Some(user) = user else {
        check_dummy_hash(password);
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

/// Check `password` against the dummy hash, for the time it takes; the answer is not used.
fn check_dummy_hash(password: &str) {
    if let Some(hash) = DUMMY_HASH.as_deref() {
        let _ = password::verify_password(password, hash);
    } else {
        tracing::error!("No dummy password hash; refused logins answer faster");
    }
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
        let user = match_password(&db, "Alice", PASSWORD).await.expect("check");
        assert_eq!(user.map(|user| user.username), Some("alice".to_owned()));
    }

    #[tokio::test]
    async fn a_wrong_password_and_an_unknown_user_look_the_same() {
        let db = database_with_alice().await;
        assert!(
            match_password(&db, "alice", "Wrong-Pass-789!")
                .await
                .expect("check")
                .is_none()
        );
        assert!(
            match_password(&db, "nobody", PASSWORD)
                .await
                .expect("check")
                .is_none()
        );
    }

    #[test]
    fn user_login_009_an_unknown_users_password_is_checked_against_a_real_hash() {
        let hash = DUMMY_HASH.as_deref().expect("dummy hash");
        assert!(password::verify_password(DUMMY_PASSWORD, hash).expect("verify"));
    }
}
