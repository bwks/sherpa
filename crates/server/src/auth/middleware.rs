use anyhow::{Context as AnyhowContext, Result};
use serde_json::Value;

use crate::auth::context::AuthContext;
use crate::auth::jwt;
use crate::daemon::state::AppState;

/// Extract and validate JWT token from RPC request params
///
/// This function:
/// 1. Extracts the "token" field from params
/// 2. Validates the token signature and expiration
/// 3. Returns an AuthContext with user info
///
/// Returns an error if:
/// - Token is missing
/// - Token is invalid or expired
/// - Token signature doesn't match
pub async fn authenticate_request(params: &Value, state: &AppState) -> Result<AuthContext> {
    // Extract token from params
    let token = params
        .get("token")
        .and_then(|v| v.as_str())
        .context("Missing 'token' field in request params")?;

    // Validate token
    let claims =
        jwt::validate_token(&state.jwt_secret, token).context("Invalid or expired token")?;

    // Create auth context from claims
    Ok(AuthContext::new(claims.sub, claims.is_admin))
}

#[cfg(test)]
mod tests {
    use super::*;
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
