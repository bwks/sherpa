use anyhow::{Context as AnyhowContext, Result, anyhow};
use serde_json::Value;

use crate::auth::context::AuthContext;
use crate::auth::jwt;
use crate::auth::session::{self, SessionLookup};
use crate::daemon::state::AppState;

/// Extract and validate JWT token from RPC request params
///
/// This function:
/// 1. Extracts the "token" field from params
/// 2. Validates the token signature and expiration
/// 3. Presents the token's session (`session::lookup`)
/// 4. Returns an `AuthContext` with the user's current name and admin flag
///
/// Returns an error if:
/// - Token is missing
/// - Token is invalid or expired
/// - Token signature doesn't match
/// - The token's session has ended or expired: logged out, ended by a password change, or its
///   user deleted
pub async fn authenticate_request(params: &Value, state: &AppState) -> Result<AuthContext> {
    // Extract token from params
    let token = params
        .get("token")
        .and_then(|v| v.as_str())
        .context("Missing 'token' field in request params")?;

    // Validate token
    let claims =
        jwt::validate_token(&state.jwt_secret, token).context("Invalid or expired token")?;

    match session::lookup(&state.db, &claims.sid).await? {
        SessionLookup::Active(ctx) => Ok(ctx),
        SessionLookup::Rejected(rejection) => Err(anyhow!(
            "Session of user '{}' cannot be used: {rejection}",
            claims.sub
        )),
    }
}
