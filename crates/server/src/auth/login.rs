//! Checking a username and password, for every login route, and the account lockout.

use std::net::IpAddr;
use std::sync::LazyLock;

use anyhow::{Context, Result};
use db::Database;
use shared::auth::password;
use shared::data::DbUser;
use tracing::instrument;

/// Checked in place of a real hash when the username does not exist or the account is locked.
const DUMMY_PASSWORD: &str = "Unknown-User-Password-1!";
/// Failed logins in a row that lock an account.
pub(crate) const LOGIN_FAILURE_LIMIT: u32 = 3;
/// The answer to every refused login, on every route: a wrong password, an unknown username
/// and a locked account alike.
pub(crate) const LOGIN_REFUSED: &str = "username and/or password is incorrect";

/// A hash made with the same parameters as real password hashes, so checking against it takes
/// as long as checking a real one.
static DUMMY_HASH: LazyLock<Option<String>> =
    LazyLock::new(|| password::hash_password(DUMMY_PASSWORD).ok());

/// The user `username`, if `password` is theirs and their account is not locked, for a login
/// from `address`. `None` means the username is unknown, the password is wrong or the account
/// is locked; callers answer all three with [`LOGIN_REFUSED`].
///
/// The attempt is counted against the account before its password is checked, so attempts
/// that arrive together check no more passwords than [`LOGIN_FAILURE_LIMIT`], and the one that
/// reaches it locks the account. A successful login sets the count back to 0. Every attempt on
/// an account that was not locked is stored with its result and `address`; an attempt on a
/// locked account and one for an unknown username are only logged. Neither checks the password
/// against a real hash, but against a dummy one, so response times do not show which usernames
/// exist or are locked. The password is never logged.
///
/// # Errors
///
/// Fails when the database cannot be read or written or a stored hash cannot be checked. The
/// attempt stays counted then, and is not stored.
#[instrument(skip(db, password), fields(%username, %address))]
pub async fn check_credentials(
    db: &Database,
    username: &str,
    password: &str,
    address: IpAddr,
) -> Result<Option<DbUser>> {
    let claim = db::claim_login(db, username, LOGIN_FAILURE_LIMIT)
        .await
        .context("Failed to count the login attempt")?;
    let Some(claim) = claim else {
        check_dummy_hash(password);
        let exists = db::find_user(db, username)
            .await
            .context("Failed to look up user")?
            .is_some();
        if exists {
            tracing::warn!(
                event = "auth.login_refused_locked",
                %username,
                %address,
                "Login refused: the account is locked"
            );
        } else {
            tracing::warn!(
                event = "auth.login_unknown_user",
                %username,
                %address,
                "Login failed: unknown user"
            );
        }
        return Ok(None);
    };
    let matched = password::verify_password(password, &claim.user.password_hash)
        .context("Failed to check the password")?;
    let stands = db::finish_login(db, &claim, matched, address)
        .await
        .context("Failed to record the login attempt")?;
    let username = &claim.user.username;
    if stands {
        tracing::info!(
            event = "auth.login_attempt_succeeded",
            %username,
            %address,
            "Login attempt succeeded"
        );
        return Ok(Some(claim.user));
    }
    if matched {
        tracing::warn!(
            event = "auth.login_refused_locked",
            %username,
            %address,
            "Login refused: the account was locked while its password was checked"
        );
    } else {
        tracing::warn!(
            event = "auth.login_failed",
            %username,
            %address,
            "Login failed: wrong password"
        );
        if claim.locked_account() {
            tracing::warn!(
                event = "auth.account_locked",
                %username,
                %address,
                failed_logins = LOGIN_FAILURE_LIMIT,
                "Account locked after failed logins in a row; an admin must unlock it"
            );
        }
    }
    Ok(None)
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
    use std::net::Ipv4Addr;

    use super::*;
    use crate::log_capture::capture_log;

    const PASSWORD: &str = "Original-Pass-123!";
    const WRONG_PASSWORD: &str = "Wrong-Pass-789!";
    const ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7));

    async fn database_with_alice() -> Database {
        let db = db::open_in_memory().await.expect("open database");
        db::apply_schema(&db).await.expect("apply schema");
        db::create_user(&db, "alice".to_owned(), PASSWORD, false, vec![])
            .await
            .expect("create user");
        db
    }

    #[test]
    fn user_login_009_an_unknown_users_password_is_checked_against_a_real_hash() {
        let hash = DUMMY_HASH.as_deref().expect("dummy hash");
        assert!(password::verify_password(DUMMY_PASSWORD, hash).expect("verify"));
    }

    #[test]
    fn user_login_023_locked_and_unknown_user_attempts_are_logged_and_not_stored() {
        // One thread, so the log of every attempt is captured.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (stored, log) = capture_log(|| {
            runtime.block_on(async {
                let db = database_with_alice().await;
                for _ in 0..LOGIN_FAILURE_LIMIT {
                    check_credentials(&db, "alice", WRONG_PASSWORD, ADDRESS)
                        .await
                        .expect("check");
                }
                let alice = db::get_user(&db, "alice").await.expect("alice");
                let id = alice.id.expect("id");
                let before = db::list_login_attempts(&db, id.clone())
                    .await
                    .expect("attempts")
                    .len();
                let locked = check_credentials(&db, "alice", PASSWORD, ADDRESS)
                    .await
                    .expect("check");
                let unknown = check_credentials(&db, "nobody", PASSWORD, ADDRESS)
                    .await
                    .expect("check");
                assert!(locked.is_none() && unknown.is_none());
                let after = db::list_login_attempts(&db, id).await.expect("attempts");
                (before, after.len())
            })
        });
        let (before, after) = stored;
        assert_eq!(before, 3, "{log}");
        assert_eq!(after, before, "a refused attempt was stored");
        let locked_line = log
            .lines()
            .find(|line| line.contains("the account is locked"))
            .unwrap_or_else(|| panic!("no locked line: {log}"));
        assert!(
            locked_line.contains("username=alice") && locked_line.contains("address=192.0.2.7"),
            "{locked_line}"
        );
        let unknown_line = log
            .lines()
            .find(|line| line.contains("unknown user"))
            .unwrap_or_else(|| panic!("no unknown user line: {log}"));
        assert!(
            unknown_line.contains("username=nobody") && unknown_line.contains("address=192.0.2.7"),
            "{unknown_line}"
        );
        assert!(
            !log.contains(PASSWORD) && !log.contains(WRONG_PASSWORD),
            "a password was logged: {log}"
        );
    }
}
