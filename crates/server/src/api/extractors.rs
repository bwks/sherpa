use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};

use crate::auth::{context::AuthContext, cookies, jwt};
use crate::daemon::state::AppState;

use super::errors::ApiError;

/// Authenticated user extracted from JWT token
///
/// Use this as a handler parameter to require authentication.
/// The token is extracted from the `Authorization: Bearer <token>` header OR from cookie.
/// Header takes priority if both are present.
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
        // Try Authorization header first (for API/CLI clients)
        if let Some(auth_header) = parts
            .headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
        {
            // Parse "Bearer <token>" format
            if let Some(token) = auth_header.strip_prefix("Bearer ") {
                // Validate token
                if let Ok(claims) = jwt::validate_token(&state.jwt_secret, token) {
                    return Ok(AuthenticatedUser {
                        username: claims.sub,
                        is_admin: claims.is_admin,
                    });
                }
            }
        }

        // Fall back to cookie (for web browser clients)
        if let Some(cookie_header) = parts
            .headers
            .get(header::COOKIE)
            .and_then(|h| h.to_str().ok())
        {
            if let Some(token) = cookies::extract_token_from_cookie(cookie_header) {
                // Validate token
                if let Ok(claims) = jwt::validate_token(&state.jwt_secret, &token) {
                    return Ok(AuthenticatedUser {
                        username: claims.sub,
                        is_admin: claims.is_admin,
                    });
                }
            }
        }

        // No valid authentication found
        Err(ApiError::unauthorized("Missing or invalid authentication"))
    }
}

/// Authenticated user extracted from cookie for HTML pages
///
/// Use this for HTML routes that should redirect to login on authentication failure.
/// This extractor ONLY checks cookies (not Authorization headers).
///
/// On authentication failure, returns a redirect to `/login?error=session_required`
///
/// # Example
/// ```rust
/// pub async fn dashboard(
///     auth: AuthenticatedUserFromCookie,
/// ) -> impl IntoResponse {
///     Html(format!("Welcome, {}!", auth.username))
/// }
/// ```
#[derive(Debug, Clone)]
pub struct AuthenticatedUserFromCookie {
    pub username: String,
    pub is_admin: bool,
}

impl AuthenticatedUserFromCookie {
    /// Convert to AuthContext for service layer
    pub fn into_context(self) -> AuthContext {
        AuthContext::new(self.username, self.is_admin)
    }
}

/// Custom rejection type that redirects to login
pub struct AuthRedirect;

impl IntoResponse for AuthRedirect {
    fn into_response(self) -> Response {
        Redirect::to("/login?error=session_required").into_response()
    }
}

impl FromRequestParts<AppState> for AuthenticatedUserFromCookie {
    type Rejection = AuthRedirect;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Extract cookie header
        let cookie_header = parts
            .headers
            .get(header::COOKIE)
            .and_then(|h| h.to_str().ok())
            .ok_or(AuthRedirect)?;

        // Extract token from cookies
        let token = cookies::extract_token_from_cookie(cookie_header).ok_or(AuthRedirect)?;

        // Validate token
        let claims = jwt::validate_token(&state.jwt_secret, &token).map_err(|e| {
            tracing::debug!("Cookie token validation failed: {}", e);
            AuthRedirect
        })?;

        Ok(AuthenticatedUserFromCookie {
            username: claims.sub,
            is_admin: claims.is_admin,
        })
    }
}
