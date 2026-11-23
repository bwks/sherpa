//! A user's device password: their login password on lab nodes, and what the API shows of it.
//!
//! Every transport calls these, so the same rules apply everywhere:
//! - only the account's own user sets it, admins included: whoever sets it knows it, so an
//!   admin cannot set, read or clear another user's;
//! - setting it needs the user's Sherpa login password, so a stolen session cannot set one;
//! - the built-in `admin` account, which cannot start labs, has none;
//! - it must meet the device password rule (`validate::validate_device_password`) and differ
//!   from the login password, checked against the stored hash;
//! - it is never returned: user details carry only whether it is set.

use anyhow::{Context, Result};
use db::Database;
use tracing::instrument;

use shared::auth::password;
use shared::data::{DbUser, SetDevicePasswordResponse, UserInfo};
use shared::util::normalize_username;

use crate::auth::context::AuthContext;
use crate::auth::policy::{require_lab_starter, require_self};
use crate::services::error::ServiceError;
use crate::validate::validate_device_password;

/// Set the caller's own device password, `username` naming the account to set it for, after
/// checking `login_password` is their Sherpa login password.
#[instrument(
    skip(ctx, login_password, device_password, db),
    fields(caller = %ctx.username, %username)
)]
pub async fn set_device_password(
    ctx: &AuthContext,
    username: &str,
    login_password: &str,
    device_password: &str,
    db: &Database,
) -> Result<SetDevicePasswordResponse> {
    let username = normalize_username(username);
    require_self(ctx, &username)?;
    require_lab_starter(ctx)?;

    let user = db::find_user(db, &username)
        .await
        .context("Failed to look up user")?
        .ok_or_else(|| ServiceError::NotFound(format!("User not found: {username}")))?;

    let login_matches = password::verify_password(login_password, &user.password_hash)
        .context("Failed to check the login password")?;
    if !login_matches {
        tracing::warn!(
            event = "user.device_password_refused",
            username = %user.username,
            "Refused a device password change: wrong login password"
        );
        return Err(ServiceError::Denied("the login password is incorrect".to_owned()).into());
    }

    validate_device_password(device_password, &user.username).map_err(|error| {
        ServiceError::Invalid(format!(
            "The device password does not meet the requirements: {error}"
        ))
    })?;
    let same_as_login = password::verify_password(device_password, &user.password_hash)
        .context("Failed to compare the device password with the login password")?;
    if same_as_login {
        return Err(ServiceError::Invalid(
            "The device password must differ from your Sherpa login password".to_owned(),
        )
        .into());
    }

    db::set_device_password(db, &db::get_user_id(&user)?, device_password)
        .await
        .context("Failed to save the device password")?;

    tracing::info!(
        event = "user.device_password_set",
        username = %user.username,
        "Device password set"
    );
    Ok(SetDevicePasswordResponse {
        username: user.username,
        device_password_set: true,
    })
}

/// The account details of `user` that the API shows: no password hash, and of the device
/// password only whether it is set.
#[instrument(skip(user, db), fields(username = %user.username), level = "debug")]
pub async fn user_info(user: DbUser, db: &Database) -> Result<UserInfo> {
    let device_password_set = db::device_password_set(db, &db::get_user_id(&user)?)
        .await
        .context("Failed to read whether the user has a device password")?;
    Ok(UserInfo {
        username: user.username,
        is_admin: user.is_admin,
        ssh_keys: user.ssh_keys,
        created_at: user.created_at.as_second(),
        updated_at: user.updated_at.as_second(),
        device_password_set,
    })
}

/// Every user's account details, as [`user_info`] gives them.
#[instrument(skip(db), level = "debug")]
pub async fn list_user_info(db: &Database) -> Result<Vec<UserInfo>> {
    let users = db::list_users(db).await.context("Failed to list users")?;
    let mut infos = Vec::with_capacity(users.len());
    for user in users {
        infos.push(user_info(user, db).await?);
    }
    Ok(infos)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN_PASSWORD: &str = "Login-Pass-123!";
    const DEVICE_PASSWORD: &str = "Granite-Owl42";

    async fn database_with_users(users: &[(&str, bool)]) -> Database {
        let db = db::open_in_memory().await.expect("open database");
        db::apply_schema(&db).await.expect("apply schema");
        for (username, is_admin) in users {
            db::create_user(
                &db,
                (*username).to_owned(),
                LOGIN_PASSWORD,
                *is_admin,
                vec![],
            )
            .await
            .expect("create user");
        }
        db
    }

    fn caller(username: &str, is_admin: bool) -> AuthContext {
        AuthContext::for_test(username, is_admin)
    }

    async fn stored(db: &Database, username: &str) -> Option<String> {
        let user = db::get_user(db, username).await.expect("user");
        db::open_device_password(db, &db::get_user_id(&user).expect("id"))
            .await
            .expect("open")
            .map(|password| password.expose().to_owned())
    }

    fn refusal(error: &anyhow::Error) -> Option<&ServiceError> {
        ServiceError::find(error)
    }

    #[tokio::test]
    async fn device_password_001_users_set_their_own_device_password_with_their_login_password() {
        let db = database_with_users(&[("alice", false)]).await;
        let alice = caller("alice", false);

        let response = set_device_password(&alice, "Alice", LOGIN_PASSWORD, DEVICE_PASSWORD, &db)
            .await
            .expect("set");
        assert_eq!(response.username, "alice");
        assert!(response.device_password_set);
        assert_eq!(stored(&db, "alice").await.as_deref(), Some(DEVICE_PASSWORD));
        let info = user_info(db::get_user(&db, "alice").await.expect("user"), &db)
            .await
            .expect("info");
        assert!(info.device_password_set);
    }

    #[tokio::test]
    async fn an_admin_cannot_set_another_users_device_password() {
        let db = database_with_users(&[("alice", false), ("dana", true)]).await;

        let error = set_device_password(
            &caller("dana", true),
            "alice",
            LOGIN_PASSWORD,
            DEVICE_PASSWORD,
            &db,
        )
        .await
        .expect_err("an admin set alice's device password");
        assert!(matches!(refusal(&error), Some(ServiceError::Denied(_))));
        assert_eq!(stored(&db, "alice").await, None);

        // An admin's own is theirs to set.
        set_device_password(
            &caller("dana", true),
            "dana",
            LOGIN_PASSWORD,
            DEVICE_PASSWORD,
            &db,
        )
        .await
        .expect("an admin sets their own");
    }

    #[tokio::test]
    async fn builtin_admin_008_the_built_in_admin_has_no_device_password() {
        let db = database_with_users(&[]).await;
        db::create_admin_user(&db, LOGIN_PASSWORD)
            .await
            .expect("admin");
        let error = set_device_password(
            &caller("sherpa-admin", true),
            "sherpa-admin",
            LOGIN_PASSWORD,
            DEVICE_PASSWORD,
            &db,
        )
        .await
        .expect_err("the built-in admin set a device password");
        assert!(matches!(refusal(&error), Some(ServiceError::Denied(_))));
        assert_eq!(stored(&db, "sherpa-admin").await, None);
    }

    #[tokio::test]
    async fn a_wrong_login_password_is_refused() {
        let db = database_with_users(&[("alice", false)]).await;
        let error = set_device_password(
            &caller("alice", false),
            "alice",
            "Wrong-Pass-789!",
            DEVICE_PASSWORD,
            &db,
        )
        .await
        .expect_err("wrong login password");
        assert!(matches!(refusal(&error), Some(ServiceError::Denied(_))));
        assert_eq!(stored(&db, "alice").await, None);
    }

    #[tokio::test]
    async fn the_login_password_cannot_be_the_device_password() {
        // A login password that also meets the device password rule.
        let db = database_with_users(&[]).await;
        db::create_user(&db, "alice".to_owned(), DEVICE_PASSWORD, false, vec![])
            .await
            .expect("create user");
        let error = set_device_password(
            &caller("alice", false),
            "alice",
            DEVICE_PASSWORD,
            DEVICE_PASSWORD,
            &db,
        )
        .await
        .expect_err("device password equal to the login password");
        assert!(matches!(refusal(&error), Some(ServiceError::Invalid(_))));
        assert!(error.to_string().contains("must differ"), "{error}");
        assert_eq!(stored(&db, "alice").await, None);
    }

    #[tokio::test]
    async fn device_password_005_a_device_password_has_12_to_20_characters() {
        let username = "alice";
        for (password, count) in [("Granite-Ow1", 11), ("Granite-Owl42-Lynx9xy", 21)] {
            let message = validate_device_password(password, username)
                .expect_err(password)
                .to_string();
            let expected = format!("12 to 20 characters; this one has {count}");
            assert!(message.contains(&expected), "{password}: {message}");
            assert!(!message.contains(password), "{password}: {message}");
        }
        validate_device_password("Granite-Ow1x", username).expect("12 characters");
        validate_device_password("Granite-Owl42-Lynx9x", username).expect("20 characters");

        let db = database_with_users(&[("alice", false)]).await;
        let error = set_device_password(
            &caller("alice", false),
            "alice",
            LOGIN_PASSWORD,
            "short",
            &db,
        )
        .await
        .expect_err("weak device password");
        assert!(matches!(refusal(&error), Some(ServiceError::Invalid(_))));
        assert_eq!(stored(&db, "alice").await, None);
    }

    #[tokio::test]
    async fn user_details_show_only_whether_it_is_set() {
        let db = database_with_users(&[("alice", false), ("bob", false)]).await;
        set_device_password(
            &caller("alice", false),
            "alice",
            LOGIN_PASSWORD,
            DEVICE_PASSWORD,
            &db,
        )
        .await
        .expect("set");
        let infos = list_user_info(&db).await.expect("list");
        let set: Vec<(&str, bool)> = infos
            .iter()
            .map(|info| (info.username.as_str(), info.device_password_set))
            .collect();
        assert_eq!(set, [("alice", true), ("bob", false)]);
        let json = serde_json::to_string(&infos).expect("json");
        assert!(!json.contains(DEVICE_PASSWORD), "{json}");
    }
}
