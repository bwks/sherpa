use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::{DbUser, DeleteUserResponse};
use shared::konst::ADMIN_USERNAME;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_admin;
use crate::daemon::state::AppState;
use crate::services::error::ServiceError;

/// Delete a user account (admin only).
///
/// Every transport calls this, so the same rules apply everywhere:
/// - only admins may delete users;
/// - an admin cannot delete their own account;
/// - the built-in `admin` account cannot be deleted (the database also refuses this);
/// - the last admin account cannot be deleted;
/// - a user who owns labs cannot be deleted until their labs are destroyed, because deleting
///   only the records would leave the labs' VMs, containers and networks running with nothing
///   to manage them. The database also rejects this (`REFERENCE ON DELETE REJECT`).
#[instrument(skip(ctx, state), fields(admin = %ctx.username, %username))]
pub async fn delete_user(
    ctx: &AuthContext,
    username: &str,
    state: &AppState,
) -> Result<DeleteUserResponse> {
    require_admin(ctx)?;

    let target = db::find_user(&state.db, username)
        .await
        .context("Failed to look up user")?
        .ok_or_else(|| ServiceError::NotFound(format!("User not found: {username}")))?;
    let target_id = db::get_user_id(&target)?;

    let users = db::list_users(&state.db)
        .await
        .context("Failed to count admin accounts")?;
    let admin_count = users.iter().filter(|user| user.is_admin).count();

    let owned_labs: Vec<String> = db::list_labs_by_user(&state.db, target_id.clone())
        .await
        .context("Failed to list the user's labs")?
        .into_iter()
        .map(|lab| format!("{} ({})", lab.lab_id, lab.name))
        .collect();

    if let Err(refusal) = check_user_delete(ctx, &target, admin_count, &owned_labs) {
        tracing::warn!(
            event = "user.delete_refused",
            admin = %ctx.username,
            target = %username,
            reason = %refusal,
            "Refused to delete user"
        );
        return Err(refusal.into());
    }

    // The schema also rejects this if the user created a lab since the check above.
    db::delete_user(&state.db, target_id)
        .await
        .context(format!(
            "Failed to delete user '{username}'. If they own labs, destroy the labs first"
        ))?;

    tracing::info!(
        event = "user.deleted",
        admin = %ctx.username,
        deleted_user = %username,
        "User deleted"
    );

    Ok(DeleteUserResponse {
        success: true,
        username: username.to_owned(),
    })
}

/// Decide whether `ctx` may delete `target`.
///
/// `admin_count` is the number of admin accounts, including `target` if it is one.
/// `owned_labs` describes the labs `target` owns.
pub fn check_user_delete(
    ctx: &AuthContext,
    target: &DbUser,
    admin_count: usize,
    owned_labs: &[String],
) -> Result<(), ServiceError> {
    require_admin(ctx)?;
    if target.username == ctx.username {
        return Err(ServiceError::Denied(
            "you cannot delete your own account".to_owned(),
        ));
    }
    if target.username == ADMIN_USERNAME {
        return Err(ServiceError::Denied(
            "the built-in admin account cannot be deleted".to_owned(),
        ));
    }
    if target.is_admin && admin_count <= 1 {
        return Err(ServiceError::Denied(
            "cannot delete the last admin account".to_owned(),
        ));
    }
    if !owned_labs.is_empty() {
        return Err(ServiceError::Conflict(format!(
            "User '{}' owns {} lab(s): {}. Destroy the labs first, then delete the user.",
            target.username,
            owned_labs.len(),
            owned_labs.join(", ")
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Timestamp;

    fn user(username: &str, is_admin: bool) -> DbUser {
        DbUser {
            id: None,
            username: username.to_owned(),
            password_hash: String::new(),
            is_admin,
            ssh_keys: vec![],
            created_at: Timestamp::UNIX_EPOCH,
            updated_at: Timestamp::UNIX_EPOCH,
        }
    }

    fn admin(username: &str) -> AuthContext {
        AuthContext::new(username.to_owned(), true)
    }

    #[test]
    fn admins_may_delete_a_user_without_labs() {
        assert!(check_user_delete(&admin("admin"), &user("carol", false), 1, &[]).is_ok());
    }

    #[test]
    fn regular_users_may_not_delete_users() {
        let ctx = AuthContext::new("bob".to_owned(), false);
        assert!(matches!(
            check_user_delete(&ctx, &user("carol", false), 1, &[]),
            Err(ServiceError::Denied(_))
        ));
    }

    #[test]
    fn admins_may_not_delete_themselves() {
        assert_eq!(
            check_user_delete(&admin("dana"), &user("dana", true), 2, &[]),
            Err(ServiceError::Denied(
                "you cannot delete your own account".to_owned()
            ))
        );
    }

    #[test]
    fn the_built_in_admin_may_not_be_deleted() {
        assert_eq!(
            check_user_delete(&admin("dana"), &user(ADMIN_USERNAME, true), 3, &[]),
            Err(ServiceError::Denied(
                "the built-in admin account cannot be deleted".to_owned()
            ))
        );
    }

    #[test]
    fn the_last_admin_may_not_be_deleted() {
        // Reachable only through a race, such as two admins deleting each other at once.
        assert_eq!(
            check_user_delete(&admin("dana"), &user("erin", true), 1, &[]),
            Err(ServiceError::Denied(
                "cannot delete the last admin account".to_owned()
            ))
        );
        assert!(check_user_delete(&admin("dana"), &user("erin", true), 2, &[]).is_ok());
    }

    #[test]
    fn users_who_own_labs_may_not_be_deleted_and_their_labs_are_listed() {
        let labs = vec!["c0ffee01 (carol-lab)".to_owned()];
        assert_eq!(
            check_user_delete(&admin("admin"), &user("carol", false), 1, &labs),
            Err(ServiceError::Conflict(
                "User 'carol' owns 1 lab(s): c0ffee01 (carol-lab). Destroy the labs first, then delete the user."
                    .to_owned()
            ))
        );
    }
}
