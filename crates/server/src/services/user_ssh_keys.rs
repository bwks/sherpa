//! Add and remove a user's SSH public keys.
//!
//! Every transport calls these, so the same rules apply everywhere: users manage their own
//! keys, and admins may manage anyone's. Labs only start for users with a registered key.

use anyhow::{Context, Result};
use db::Database;
use tracing::instrument;

use shared::data::{DbUser, UserSshKeysResponse};
use shared::util::{normalize_username, ssh_key_fingerprint};
use shared::validators::validate_ssh_key;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_self_or_admin;
use crate::daemon::state::AppState;
use crate::services::error::ServiceError;

/// Register an SSH public key for `username`.
#[instrument(skip(ctx, ssh_key, state), fields(caller = %ctx.username, %username))]
pub async fn add_ssh_key(
    ctx: &AuthContext,
    username: &str,
    ssh_key: &str,
    state: &AppState,
) -> Result<UserSshKeysResponse> {
    let username = normalize_username(username);
    require_self_or_admin(ctx, &username)?;

    let ssh_key = ssh_key.trim();
    validate_ssh_key(ssh_key).context("Invalid SSH public key")?;

    let mut user = load_user(&username, state).await?;
    if find_ssh_key(&user.ssh_keys, ssh_key).is_some() {
        return Err(ServiceError::Conflict(format!(
            "This SSH key is already registered for '{username}'"
        ))
        .into());
    }
    user.ssh_keys.push(ssh_key.to_owned());
    let response = save_keys(user, &state.db).await?;

    tracing::info!(
        event = "user.ssh_key_added",
        caller = %ctx.username,
        %username,
        "SSH key added"
    );
    Ok(response)
}

/// Remove the SSH public key matching `ssh_key` (the key or its `SHA256:` fingerprint).
#[instrument(skip(ctx, ssh_key, state), fields(caller = %ctx.username, %username))]
pub async fn remove_ssh_key(
    ctx: &AuthContext,
    username: &str,
    ssh_key: &str,
    state: &AppState,
) -> Result<UserSshKeysResponse> {
    let username = normalize_username(username);
    require_self_or_admin(ctx, &username)?;

    let mut user = load_user(&username, state).await?;
    let index = find_ssh_key(&user.ssh_keys, ssh_key).ok_or_else(|| {
        ServiceError::NotFound(format!(
            "No matching SSH key is registered for '{username}'"
        ))
    })?;
    user.ssh_keys.remove(index);
    let response = save_keys(user, &state.db).await?;

    tracing::info!(
        event = "user.ssh_key_removed",
        caller = %ctx.username,
        %username,
        "SSH key removed"
    );
    Ok(response)
}

async fn load_user(username: &str, state: &AppState) -> Result<DbUser> {
    Ok(db::find_user(&state.db, username)
        .await
        .context("Failed to look up user")?
        .ok_or_else(|| ServiceError::NotFound(format!("User not found: {username}")))?)
}

/// Save `user`'s keys. A concurrent edit of the same user since it was read is refused as a
/// conflict, not overwritten.
#[instrument(level = "debug", skip(user, db), fields(username = %user.username))]
async fn save_keys(user: DbUser, db: &Database) -> Result<UserSshKeysResponse> {
    let user = db::update_user(db, user)
        .await
        .map_err(ServiceError::from_user_write)
        .context("Failed to save SSH keys")?;
    Ok(UserSshKeysResponse {
        username: user.username,
        ssh_keys: user.ssh_keys,
    })
}

/// Index of the registered key matching `target`: the same key data, whatever its comment, or
/// a key whose `SHA256:` fingerprint is `target`.
fn find_ssh_key(keys: &[String], target: &str) -> Option<usize> {
    let target = target.trim();
    let fingerprint = if target.starts_with("SHA256:") {
        Some(target.to_owned())
    } else {
        ssh_key_fingerprint(target).ok()
    };
    keys.iter().position(|key| match &fingerprint {
        Some(fingerprint) => ssh_key_fingerprint(key).ok().as_ref() == Some(fingerprint),
        None => key.trim() == target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh";
    const FINGERPRINT: &str = "SHA256:/UpBoWzMiq6/+/wzQIGDT2VxOLzg/LUTfKhc0x+ktEs";

    #[test]
    fn keys_match_by_key_data_ignoring_comments() {
        let keys = vec!["ssh-rsa legacy".to_owned(), format!("{KEY} laptop")];
        assert_eq!(find_ssh_key(&keys, KEY), Some(1));
        assert_eq!(find_ssh_key(&keys, &format!("  {KEY} desktop ")), Some(1));
    }

    #[test]
    fn keys_match_by_fingerprint() {
        let keys = vec![format!("{KEY} laptop")];
        assert_eq!(find_ssh_key(&keys, FINGERPRINT), Some(0));
        assert_eq!(find_ssh_key(&keys, "SHA256:unknown"), None);
    }

    #[tokio::test]
    async fn a_key_edit_from_a_stale_read_is_a_conflict_and_keeps_the_other_edit() {
        let db = db::open_in_memory().await.expect("open database");
        db::apply_schema(&db).await.expect("apply schema");
        let read = db::create_user(&db, "alice".to_owned(), "Original-Pass-123!", false, vec![])
            .await
            .expect("create user");
        let mut first = read.clone();
        first.ssh_keys.push(format!("{KEY} laptop"));
        let mut second = read;
        second.ssh_keys.push("ssh-rsa desktop".to_owned());

        save_keys(first, &db).await.expect("first edit");
        let error = save_keys(second, &db).await.expect_err("stale edit saved");

        assert!(
            matches!(ServiceError::find(&error), Some(ServiceError::Conflict(_))),
            "{error:#}"
        );
        let stored = db::get_user(&db, "alice").await.expect("read");
        assert_eq!(stored.ssh_keys, vec![format!("{KEY} laptop")]);
    }

    #[test]
    fn unparseable_keys_match_only_exactly() {
        let keys = vec!["ssh-rsa legacy".to_owned()];
        assert_eq!(find_ssh_key(&keys, "ssh-rsa legacy"), Some(0));
        assert_eq!(find_ssh_key(&keys, "ssh-rsa other"), None);
    }
}
