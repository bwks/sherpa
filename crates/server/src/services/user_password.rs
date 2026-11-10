use anyhow::{Context, Result};
use db::Database;
use tracing::instrument;

use shared::auth::password;
use shared::data::ChangePasswordResponse;
use shared::util::normalize_username;
use shared::validators::validate_password_strength;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_self_or_admin;
use crate::services::error::ServiceError;

/// Change the password of `username`.
///
/// Every transport calls this, so the same rules apply everywhere:
/// - users change only their own password; admins may reset anyone's;
/// - changing your own password, as an admin too, needs the current password, so a stolen
///   token or an unlocked session cannot lock the owner out;
/// - an admin resetting another user's password does not need that user's password;
/// - the new password must meet the strength rules;
/// - the change ends the user's sessions: all of them but the caller's own when users change
///   their own password, and all of them when an admin resets someone else's. The hash and the
///   sessions are written in one transaction.
#[instrument(skip(ctx, current_password, new_password, db), fields(caller = %ctx.username, %username))]
pub async fn change_password(
    ctx: &AuthContext,
    username: &str,
    current_password: Option<&str>,
    new_password: &str,
    db: &Database,
) -> Result<ChangePasswordResponse> {
    let username = normalize_username(username);
    require_self_or_admin(ctx, &username)?;

    let user = db::find_user(db, &username)
        .await
        .context("Failed to look up user")?
        .ok_or_else(|| ServiceError::NotFound(format!("User not found: {username}")))?;

    if ctx.username == user.username {
        let current = current_password.ok_or_else(|| {
            ServiceError::Invalid("The current password is required to change your own".to_owned())
        })?;
        let matches = password::verify_password(current, &user.password_hash)
            .context("Failed to check the current password")?;
        if !matches {
            tracing::warn!(
                event = "user.password_change_refused",
                username = %user.username,
                "Refused a password change: wrong current password"
            );
            return Err(
                ServiceError::Denied("the current password is incorrect".to_owned()).into(),
            );
        }
    }

    validate_password_strength(new_password).map_err(|error| {
        ServiceError::Invalid(format!(
            "The new password does not meet the requirements: {error}"
        ))
    })?;
    let password_hash =
        password::hash_password(new_password).context("Failed to hash the new password")?;
    // Users keep the session they changed it from; an admin's reset ends all of the target's.
    let keep = (ctx.username == user.username).then_some(ctx.session_id.as_str());
    let sessions_ended =
        db::change_user_password(db, db::get_user_id(&user)?, &password_hash, keep)
            .await
            .context("Failed to save the new password")?;

    tracing::info!(
        event = "user.password_changed",
        caller = %ctx.username,
        target = %username,
        sessions_ended,
        "Password changed"
    );
    Ok(ChangePasswordResponse {
        username,
        success: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use shared::data::SessionChannel;

    use crate::auth::session::{self, SessionLookup};

    const OLD_PASSWORD: &str = "Original-Pass-123!";
    const NEW_PASSWORD: &str = "Changed-Pass-456!";

    async fn database_with_users(users: &[(&str, bool)]) -> Database {
        let db = db::open_in_memory().await.expect("open database");
        db::apply_schema(&db).await.expect("apply schema");
        for (username, is_admin) in users {
            db::create_user(&db, (*username).to_owned(), OLD_PASSWORD, *is_admin, vec![])
                .await
                .expect("create user");
        }
        db
    }

    fn caller(username: &str, is_admin: bool) -> AuthContext {
        AuthContext::for_test(username, is_admin)
    }

    async fn password_is(db: &Database, username: &str, candidate: &str) -> bool {
        let user = db::get_user(db, username).await.expect("user");
        password::verify_password(candidate, &user.password_hash).expect("verify")
    }

    fn service_error(error: &anyhow::Error) -> Option<&ServiceError> {
        ServiceError::find(error)
    }

    #[tokio::test]
    async fn your_own_password_needs_the_current_one() {
        let db = database_with_users(&[("alice", false)]).await;
        let alice = caller("alice", false);

        let missing = change_password(&alice, "alice", None, NEW_PASSWORD, &db)
            .await
            .expect_err("no current password");
        assert!(matches!(
            service_error(&missing),
            Some(ServiceError::Invalid(_))
        ));

        let wrong = change_password(&alice, "alice", Some("Wrong-Pass-789!"), NEW_PASSWORD, &db)
            .await
            .expect_err("wrong current password");
        assert!(matches!(
            service_error(&wrong),
            Some(ServiceError::Denied(_))
        ));
        assert!(password_is(&db, "alice", OLD_PASSWORD).await);

        change_password(&alice, "Alice", Some(OLD_PASSWORD), NEW_PASSWORD, &db)
            .await
            .expect("changed");
        assert!(password_is(&db, "alice", NEW_PASSWORD).await);
    }

    #[tokio::test]
    async fn an_admin_resets_others_without_their_password_but_not_themselves() {
        let db = database_with_users(&[("alice", false), ("dana", true)]).await;
        let dana = caller("dana", true);

        change_password(&dana, "alice", None, NEW_PASSWORD, &db)
            .await
            .expect("reset");
        assert!(password_is(&db, "alice", NEW_PASSWORD).await);

        change_password(&dana, "dana", None, NEW_PASSWORD, &db)
            .await
            .expect_err("own password without the current one");
        assert!(password_is(&db, "dana", OLD_PASSWORD).await);
    }

    /// Start a session for `username`, returning its id.
    async fn session_of(db: &Database, username: &str) -> String {
        let user = db::get_user(db, username).await.expect("user");
        session::start(
            db,
            b"test_secret_32_bytes_long_enough",
            &user,
            SessionChannel::Cli,
            3600,
        )
        .await
        .expect("start session")
        .ctx
        .session_id
    }

    async fn is_active(db: &Database, session_id: &str) -> bool {
        matches!(
            session::lookup(db, session_id).await.expect("lookup"),
            SessionLookup::Active(_)
        )
    }

    #[tokio::test]
    async fn changing_your_own_password_ends_your_other_sessions_only() {
        let db = database_with_users(&[("alice", false)]).await;
        let current = session_of(&db, "alice").await;
        let other = session_of(&db, "alice").await;
        let alice = AuthContext::new("alice".to_owned(), false, current.clone());

        change_password(&alice, "alice", Some(OLD_PASSWORD), NEW_PASSWORD, &db)
            .await
            .expect("changed");

        assert!(is_active(&db, &current).await, "the caller's session ended");
        assert!(!is_active(&db, &other).await, "another session survived");
    }

    #[tokio::test]
    async fn an_admin_reset_ends_every_session_of_the_user_and_none_of_the_admins() {
        let db = database_with_users(&[("alice", false), ("dana", true)]).await;
        let alice_session = session_of(&db, "alice").await;
        let dana_session = session_of(&db, "dana").await;
        let dana = AuthContext::new("dana".to_owned(), true, dana_session.clone());

        change_password(&dana, "alice", None, NEW_PASSWORD, &db)
            .await
            .expect("reset");

        assert!(!is_active(&db, &alice_session).await);
        assert!(is_active(&db, &dana_session).await);
    }

    #[tokio::test]
    async fn a_refused_change_ends_no_session() {
        let db = database_with_users(&[("alice", false)]).await;
        let other = session_of(&db, "alice").await;

        change_password(
            &caller("alice", false),
            "alice",
            Some("Wrong-Pass-789!"),
            NEW_PASSWORD,
            &db,
        )
        .await
        .expect_err("wrong current password");

        assert!(is_active(&db, &other).await);
    }

    #[tokio::test]
    async fn a_user_cannot_change_someone_elses_password() {
        let db = database_with_users(&[("alice", false), ("bob", false)]).await;

        let error = change_password(
            &caller("bob", false),
            "alice",
            Some(OLD_PASSWORD),
            NEW_PASSWORD,
            &db,
        )
        .await
        .expect_err("bob changed alice's password");
        assert!(matches!(
            service_error(&error),
            Some(ServiceError::Denied(_))
        ));
        assert!(password_is(&db, "alice", OLD_PASSWORD).await);
    }

    #[tokio::test]
    async fn a_weak_new_password_is_refused() {
        let db = database_with_users(&[("alice", false)]).await;

        let error = change_password(
            &caller("alice", false),
            "alice",
            Some(OLD_PASSWORD),
            "short",
            &db,
        )
        .await
        .expect_err("weak password accepted");
        assert!(matches!(
            service_error(&error),
            Some(ServiceError::Invalid(_))
        ));
        assert!(password_is(&db, "alice", OLD_PASSWORD).await);
    }
}
