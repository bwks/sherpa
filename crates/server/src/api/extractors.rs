use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};

use crate::auth::{context::AuthContext, cookies, jwt};
use crate::daemon::state::AppState;

use super::errors::ApiError;

// Extractor logic helper: extract and validate token from Authorization header or Cookie
fn extract_and_validate_token(
    headers: &axum::http::HeaderMap,
    jwt_secret: &[u8],
) -> Result<(String, bool), &'static str> {
    // Try Authorization header first
    if let Some(auth_header) = headers.get("authorization").and_then(|h| h.to_str().ok())
        && let Some(token) = auth_header.strip_prefix("Bearer ")
        && let Ok(claims) = jwt::validate_token(jwt_secret, token)
    {
        return Ok((claims.sub, claims.is_admin));
    }

    // Fall back to cookie
    if let Some(cookie_header) = headers.get(header::COOKIE).and_then(|h| h.to_str().ok())
        && let Some(token) = cookies::extract_token_from_cookie(cookie_header)
        && let Ok(claims) = jwt::validate_token(jwt_secret, &token)
    {
        return Ok((claims.sub, claims.is_admin));
    }

    Err("Missing or invalid authentication")
}

/// Authenticated user extracted from JWT token.
///
/// Use this as a handler parameter to require authentication.
/// The token is extracted from the `Authorization: Bearer <token>` header OR from cookie.
/// Header takes priority if both are present.
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub username: String,
    pub is_admin: bool,
}

impl AuthenticatedUser {
    /// Convert to AuthContext for service layer
    #[allow(dead_code)]
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
        let (username, is_admin) = extract_and_validate_token(&parts.headers, &state.jwt_secret)
            .map_err(ApiError::unauthorized)?;
        Ok(AuthenticatedUser { username, is_admin })
    }
}

/// Authenticated user extracted from cookie for HTML pages.
///
/// Use this for HTML routes that should redirect to login on authentication failure.
/// This extractor ONLY checks cookies (not Authorization headers).
/// On authentication failure, returns a redirect to `/login?error=session_required`.
#[derive(Debug, Clone)]
pub struct AuthenticatedUserFromCookie {
    pub username: String,
    pub is_admin: bool,
}

impl AuthenticatedUserFromCookie {
    /// Convert to AuthContext for service layer
    #[allow(dead_code)]
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

/// Admin user extracted from cookie for admin-only HTML pages.
///
/// Use this for HTML routes that require admin privileges.
/// This extractor checks for valid authentication AND admin status.
/// On authentication failure, redirects to `/login?error=session_required`.
/// On non-admin access, returns a 403 Forbidden page.
#[derive(Debug, Clone)]
pub struct AdminUser {
    pub username: String,
}

impl AdminUser {
    /// Convert to AuthContext for service layer
    #[allow(dead_code)]
    pub fn into_context(self) -> AuthContext {
        AuthContext::new(self.username, true)
    }
}

/// Custom rejection type for non-admin access (403 Forbidden)
pub struct AdminForbidden {
    pub username: String,
}

impl IntoResponse for AdminForbidden {
    fn into_response(self) -> Response {
        use crate::templates::Admin403Template;
        Admin403Template {
            username: self.username,
            is_admin: false,
            active_page: String::new(),
        }
        .into_response()
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Extract cookie header
        let cookie_header = parts
            .headers
            .get(header::COOKIE)
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| AuthRedirect.into_response())?;

        // Extract token from cookies
        let token = cookies::extract_token_from_cookie(cookie_header)
            .ok_or_else(|| AuthRedirect.into_response())?;

        // Validate token
        let claims = jwt::validate_token(&state.jwt_secret, &token).map_err(|e| {
            tracing::debug!("Cookie token validation failed: {}", e);
            AuthRedirect.into_response()
        })?;

        // Check if user is admin
        if !claims.is_admin {
            tracing::warn!(
                "User {} attempted to access admin route without privileges",
                claims.sub
            );
            return Err(AdminForbidden {
                username: claims.sub,
            }
            .into_response());
        }

        Ok(AdminUser {
            username: claims.sub,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    const TEST_SECRET: &[u8] = b"test_secret_32_bytes_long_enough";

    fn make_token(username: &str, is_admin: bool) -> String {
        jwt::create_token(TEST_SECRET, username, is_admin, 3600)
            .expect("Failed to create test token")
    }

    fn make_expired_token() -> String {
        use jsonwebtoken::{EncodingKey, Header, encode};
        use shared::auth::jwt::Claims;

        let now = jiff::Timestamp::now().as_second();
        let claims = Claims {
            sub: "expired_user".to_string(),
            exp: now - 100,
            iat: now - 200,
            is_admin: false,
        };
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(TEST_SECRET),
        )
        .expect("Failed to encode expired token")
    }

    // ============================================================================
    // Bearer token extraction
    // ============================================================================

    #[test]
    fn test_valid_bearer_token() {
        let token = make_token("alice", false);
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", token).parse().unwrap(),
        );

        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_ok());
        let (username, is_admin) = result.unwrap();
        assert_eq!(username, "alice");
        assert!(!is_admin);
    }

    #[test]
    fn test_valid_bearer_token_admin() {
        let token = make_token("admin", true);
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", token).parse().unwrap(),
        );

        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_ok());
        let (username, is_admin) = result.unwrap();
        assert_eq!(username, "admin");
        assert!(is_admin);
    }

    #[test]
    fn test_expired_bearer_token_rejected() {
        let token = make_expired_token();
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", token).parse().unwrap(),
        );

        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_err());
    }

    #[test]
    fn test_malformed_bearer_token_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer not.a.valid.jwt".parse().unwrap());

        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_secret_bearer_token_rejected() {
        let token = make_token("alice", false);
        let wrong_secret = b"different_secret_32bytes_exactly";
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", token).parse().unwrap(),
        );

        let result = extract_and_validate_token(&headers, wrong_secret);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_bearer_prefix_rejected() {
        let token = make_token("alice", false);
        let mut headers = HeaderMap::new();
        headers.insert("authorization", token.parse().unwrap());

        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_err());
    }

    // ============================================================================
    // Cookie token extraction
    // ============================================================================

    #[test]
    fn test_valid_cookie_token() {
        let token = make_token("bob", false);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("sherpa_auth={}", token).parse().unwrap(),
        );

        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_ok());
        let (username, _) = result.unwrap();
        assert_eq!(username, "bob");
    }

    #[test]
    fn test_cookie_among_multiple_cookies() {
        let token = make_token("charlie", true);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("other=val; sherpa_auth={}; another=x", token)
                .parse()
                .unwrap(),
        );

        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_ok());
        let (username, is_admin) = result.unwrap();
        assert_eq!(username, "charlie");
        assert!(is_admin);
    }

    #[test]
    fn test_expired_cookie_token_rejected() {
        let token = make_expired_token();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("sherpa_auth={}", token).parse().unwrap(),
        );

        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_err());
    }

    // ============================================================================
    // Precedence and missing auth
    // ============================================================================

    #[test]
    fn test_bearer_takes_precedence_over_cookie() {
        let bearer_token = make_token("bearer_user", true);
        let cookie_token = make_token("cookie_user", false);
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", bearer_token).parse().unwrap(),
        );
        headers.insert(
            header::COOKIE,
            format!("sherpa_auth={}", cookie_token).parse().unwrap(),
        );

        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_ok());
        let (username, is_admin) = result.unwrap();
        assert_eq!(username, "bearer_user");
        assert!(is_admin);
    }

    #[test]
    fn test_falls_back_to_cookie_when_bearer_invalid() {
        let cookie_token = make_token("cookie_user", false);
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer invalid.jwt.token".parse().unwrap());
        headers.insert(
            header::COOKIE,
            format!("sherpa_auth={}", cookie_token).parse().unwrap(),
        );

        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_ok());
        let (username, _) = result.unwrap();
        assert_eq!(username, "cookie_user");
    }

    #[test]
    fn test_no_auth_headers_rejected() {
        let headers = HeaderMap::new();
        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_cookie_name_rejected() {
        let token = make_token("alice", false);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("wrong_cookie={}", token).parse().unwrap(),
        );

        let result = extract_and_validate_token(&headers, TEST_SECRET);
        assert!(result.is_err());
    }
}
