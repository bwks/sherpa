use anyhow::Result;
use tracing::instrument;

use shared::data::UnlockUserResponse;
use shared::util::normalize_username;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_admin;
use crate::services::error::ServiceError;

/// Unlock the account `username`, which failed logins locked, and set its failed login count
/// back to 0 (admin only). Every transport calls this. Unlocking an account that is not locked
/// is not an error; an unknown user is not found.
#[instrument(skip(ctx, db), fields(admin = %ctx.username, %username))]
pub async fn unlock_user(
    ctx: &AuthContext,
    username: &str,
    db: &db::Database,
) -> Result<UnlockUserResponse> {
    require_admin(ctx)?;
    let username = normalize_username(username);
    if !db::unlock_user(db, &username).await? {
        return Err(ServiceError::NotFound(format!("User not found: {username}")).into());
    }
    tracing::info!(
        event = "user.unlocked",
        admin = %ctx.username,
        unlocked_user = %username,
        "Account unlocked"
    );
    Ok(UnlockUserResponse {
        success: true,
        username,
    })
}
