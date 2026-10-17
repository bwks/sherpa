use anyhow::{Context as AnyhowContext, Result, anyhow};
use db::Database;
use serde_json::Value;
use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::jwt;
use crate::daemon::state::AppState;

/// Resolve a verified token subject to the current user record.
///
/// Returns `None` when the user no longer exists. The admin flag comes from the database,
/// not from the token, so demoting or deleting a user takes effect on their next request
/// instead of when their token expires.
#[instrument(level = "debug", skip(db))]
pub async fn resolve_user(db: &Database, username: &str) -> Result<Option<AuthContext>> {
    let user = db::find_user(db, username)
        .await
        .context("Failed to look up authenticated user")?;
    Ok(user.map(|user| AuthContext::new(user.username, user.is_admin)))
}

/// Extract and validate JWT token from RPC request params
///
/// This function:
/// 1. Extracts the "token" field from params
/// 2. Validates the token signature and expiration
/// 3. Resolves the token's user against the database
/// 4. Returns an AuthContext with the user's current admin flag
///
/// Returns an error if:
/// - Token is missing
/// - Token is invalid or expired
/// - Token signature doesn't match
/// - The user no longer exists
pub async fn authenticate_request(params: &Value, state: &AppState) -> Result<AuthContext> {
    // Extract token from params
    let token = params
        .get("token")
        .and_then(|v| v.as_str())
        .context("Missing 'token' field in request params")?;

    // Validate token
    let claims =
        jwt::validate_token(&state.jwt_secret, token).context("Invalid or expired token")?;

    resolve_user(&state.db, &claims.sub)
        .await?
        .ok_or_else(|| anyhow!("User '{}' no longer exists", claims.sub))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    // Note: authenticate_request() tests require full AppState infrastructure (DB, Docker, libvirt, etc.)
    // For simpler unit tests, see jwt.rs and context.rs
    // Integration tests should cover the full authentication flow

    #[test]
    fn test_token_extraction_logic() {
        // Test the logic of extracting token from params
        let params = json!({"token": "some.jwt.token", "lab_id": "lab1"});
        let token = params.get("token").and_then(|v| v.as_str());
        assert_eq!(token, Some("some.jwt.token"));

        let params_no_token = json!({"lab_id": "lab1"});
        let token = params_no_token.get("token").and_then(|v| v.as_str());
        assert_eq!(token, None);
    }
}
