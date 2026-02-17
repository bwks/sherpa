use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::auth::{context::AuthContext, jwt};
use crate::daemon::state::AppState;

use super::errors::ApiError;

/// Authenticated user extracted from JWT token
///
/// Use this as a handler parameter to require authentication.
/// The token is extracted from the `Authorization: Bearer <token>` header.
///
/// # Example
/// ```rust
/// pub async fn get_lab(
///     Path(lab_id): Path<String>,
///     auth: AuthenticatedUser,  // ← Automatic authentication!
/// ) -> Result<Json<Response>, ApiError> {
///     // auth.username is guaranteed valid
///     println!("User {} is accessing lab {}", auth.username, lab_id);
///     Ok(Json(response))
/// }
/// ```
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub username: String,
    pub is_admin: bool,
}

impl AuthenticatedUser {
    /// Convert to AuthContext for service layer
    pub fn into_context(self) -> AuthContext {
        AuthContext::new(self.username, self.is_admin)
    }
}

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Extract Authorization header
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| ApiError::unauthorized("Missing Authorization header"))?;

        // Parse "Bearer <token>" format
        let token = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
            ApiError::unauthorized(
                "Invalid Authorization header format. Expected: Authorization: Bearer <token>",
            )
        })?;

        // Validate token
        let claims = jwt::validate_token(&state.jwt_secret, token).map_err(|e| {
            tracing::debug!("Token validation failed: {}", e);
            ApiError::unauthorized("Invalid or expired token")
        })?;

        Ok(AuthenticatedUser {
            username: claims.sub,
            is_admin: claims.is_admin,
        })
    }
}
