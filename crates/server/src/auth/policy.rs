//! Authorization rules shared by every service.
//!
//! Services call these helpers with the caller's [`AuthContext`] before they touch any
//! resource. Transports authenticate the caller and pass the context in; they do not repeat
//! the rules. A transport that must answer before the service runs (a streaming response, or
//! work done before the service is called) may call the same helper first, so the rule still
//! lives in one place.
//!
//! Failures are returned as [`ServiceError`] inside `anyhow::Error`, so transports can map them
//! to their own status codes by downcasting instead of matching on error text.

use anyhow::{Context, Result, anyhow};
use db::Database;
use shared::data::DbLab;
use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::services::error::ServiceError;

/// Which callers may act on a lab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabRule {
    /// The lab owner, or any admin.
    OwnerOrAdmin,
    /// The lab owner only, even for admins.
    OwnerOnly,
}

/// Decide whether the caller may act on a lab owned by `owner_username`.
pub fn check_lab_rule(
    ctx: &AuthContext,
    rule: LabRule,
    lab_id: &str,
    owner_username: &str,
) -> Result<(), ServiceError> {
    let allowed = match rule {
        LabRule::OwnerOrAdmin => ctx.can_access(owner_username),
        LabRule::OwnerOnly => ctx.can_redeploy(owner_username),
    };
    if allowed {
        return Ok(());
    }
    let message = match rule {
        LabRule::OwnerOrAdmin => format!("Lab '{lab_id}' is owned by another user"),
        LabRule::OwnerOnly => {
            format!("Only the owner of lab '{lab_id}' can perform this operation")
        }
    };
    Err(ServiceError::Denied(message))
}

/// Require admin privileges.
pub fn require_admin(ctx: &AuthContext) -> Result<(), ServiceError> {
    if ctx.is_admin {
        return Ok(());
    }
    tracing::warn!(
        event = "auth.access_denied",
        username = %ctx.username,
        "Non-admin attempted an admin-only operation"
    );
    Err(ServiceError::Denied("admin privileges required".to_owned()))
}

/// Require that the caller is `username`, or an admin.
pub fn require_self_or_admin(ctx: &AuthContext, username: &str) -> Result<(), ServiceError> {
    if ctx.can_access(username) {
        return Ok(());
    }
    tracing::warn!(
        event = "auth.access_denied",
        username = %ctx.username,
        target_user = %username,
        "User attempted to act on another user's account"
    );
    Err(ServiceError::Denied(format!(
        "cannot act on behalf of user '{username}'"
    )))
}

/// Require that the caller owns the lab or is an admin. Returns the lab.
#[instrument(level = "debug", skip(ctx, db), fields(username = %ctx.username, %lab_id))]
pub async fn require_lab_access(ctx: &AuthContext, db: &Database, lab_id: &str) -> Result<DbLab> {
    authorize_lab(ctx, db, lab_id, LabRule::OwnerOrAdmin).await
}

/// Require that the caller owns the lab, with no admin bypass. Returns the lab.
#[instrument(level = "debug", skip(ctx, db), fields(username = %ctx.username, %lab_id))]
pub async fn require_lab_owner(ctx: &AuthContext, db: &Database, lab_id: &str) -> Result<DbLab> {
    authorize_lab(ctx, db, lab_id, LabRule::OwnerOnly).await
}

async fn authorize_lab(
    ctx: &AuthContext,
    db: &Database,
    lab_id: &str,
    rule: LabRule,
) -> Result<DbLab> {
    let lab = db::find_lab(db, lab_id)
        .await
        .context("Failed to look up lab for authorization")?
        .ok_or_else(|| ServiceError::NotFound(format!("Lab not found: {lab_id}")))?;
    let owner = db::get_user_by_id(db, lab.user.clone())
        .await
        .context("Failed to look up lab owner for authorization")?
        .ok_or_else(|| anyhow!("Owner of lab '{lab_id}' not found"))?;

    if let Err(error) = check_lab_rule(ctx, rule, lab_id, &owner.username) {
        tracing::warn!(
            event = "auth.access_denied",
            username = %ctx.username,
            lab_id = %lab_id,
            owner = %owner.username,
            "User attempted to act on another user's lab"
        );
        return Err(error.into());
    }

    if ctx.username != owner.username {
        tracing::warn!(
            event = "auth.admin_lab_access",
            admin = %ctx.username,
            lab_id = %lab_id,
            owner = %owner.username,
            "Admin acting on another user's lab"
        );
    }

    Ok(lab)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(username: &str, is_admin: bool) -> AuthContext {
        AuthContext::new(username.to_owned(), is_admin)
    }

    #[test]
    fn owner_or_admin_allows_the_owner_and_admins_only() {
        let rule = LabRule::OwnerOrAdmin;
        assert!(check_lab_rule(&ctx("alice", false), rule, "lab1", "alice").is_ok());
        assert!(check_lab_rule(&ctx("admin", true), rule, "lab1", "alice").is_ok());
        assert_eq!(
            check_lab_rule(&ctx("bob", false), rule, "lab1", "alice"),
            Err(ServiceError::Denied(
                "Lab 'lab1' is owned by another user".to_owned()
            ))
        );
    }

    #[test]
    fn owner_only_refuses_admins_who_do_not_own_the_lab() {
        let rule = LabRule::OwnerOnly;
        assert!(check_lab_rule(&ctx("alice", false), rule, "lab1", "alice").is_ok());
        assert!(check_lab_rule(&ctx("admin", true), rule, "lab1", "admin").is_ok());
        assert!(matches!(
            check_lab_rule(&ctx("admin", true), rule, "lab1", "alice"),
            Err(ServiceError::Denied(_))
        ));
        assert!(matches!(
            check_lab_rule(&ctx("bob", false), rule, "lab1", "alice"),
            Err(ServiceError::Denied(_))
        ));
    }

    #[test]
    fn require_admin_refuses_regular_users() {
        assert!(require_admin(&ctx("admin", true)).is_ok());
        assert!(matches!(
            require_admin(&ctx("alice", false)),
            Err(ServiceError::Denied(_))
        ));
    }

    #[test]
    fn require_self_or_admin_allows_self_and_admins() {
        assert!(require_self_or_admin(&ctx("alice", false), "alice").is_ok());
        assert!(require_self_or_admin(&ctx("admin", true), "alice").is_ok());
        assert!(matches!(
            require_self_or_admin(&ctx("bob", false), "alice"),
            Err(ServiceError::Denied(_))
        ));
    }
}
