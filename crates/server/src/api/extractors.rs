use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};
use jsonwebtoken::errors::{Error as JwtError, ErrorKind};
use shared::auth::jwt::Claims;

use crate::auth::{context::AuthContext, cookies, jwt};
use crate::daemon::state::AppState;

use super::errors::ApiError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionFailureReason {
    Expired,
    InvalidSignature,
    Malformed,
    InvalidClaims,
}

impl SessionFailureReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::InvalidSignature => "invalid_signature",
            Self::Malformed => "malformed",
            Self::InvalidClaims => "invalid_claims",
        }
    }

    fn is_suspicious(self) -> bool {
        matches!(self, Self::InvalidSignature | Self::Malformed)
    }
}

#[derive(Debug)]
pub(super) struct RejectedSessionCandidate {
    index: usize,
    reason: SessionFailureReason,
    signed_claims: Option<Claims>,
}

#[derive(Debug)]
pub(super) struct ValidatedCookieSession {
    pub(super) claims: Claims,
    pub(super) candidate_count: usize,
    pub(super) selected_index: usize,
    rejected_candidates: Vec<RejectedSessionCandidate>,
}

#[derive(Debug)]
pub(super) enum CookieSessionError {
    Missing,
    Invalid {
        candidate_count: usize,
        rejected_candidates: Vec<RejectedSessionCandidate>,
    },
}

fn classify_session_failure(error: &anyhow::Error) -> SessionFailureReason {
    let Some(jwt_error) = error.downcast_ref::<JwtError>() else {
        return SessionFailureReason::InvalidClaims;
    };

    match jwt_error.kind() {
        ErrorKind::ExpiredSignature => SessionFailureReason::Expired,
        ErrorKind::InvalidSignature => SessionFailureReason::InvalidSignature,
        ErrorKind::InvalidToken
        | ErrorKind::InvalidAlgorithm
        | ErrorKind::InvalidAlgorithmName
        | ErrorKind::MissingAlgorithm
        | ErrorKind::Base64(_)
        | ErrorKind::Json(_)
        | ErrorKind::Utf8(_) => SessionFailureReason::Malformed,
        _ => SessionFailureReason::InvalidClaims,
    }
}

#[tracing::instrument(level = "debug", skip(cookie_header, jwt_secret))]
pub(super) fn validate_cookie_session(
    cookie_header: Option<&str>,
    jwt_secret: &[u8],
) -> Result<ValidatedCookieSession, CookieSessionError> {
    let cookie_header = cookie_header.ok_or(CookieSessionError::Missing)?;
    let tokens = cookies::extract_tokens_from_cookie(cookie_header);
    if tokens.is_empty() {
        return Err(CookieSessionError::Missing);
    }

    let candidate_count = tokens.len();
    let mut rejected_candidates = Vec::new();

    for (index, token) in tokens.into_iter().enumerate() {
        match jwt::validate_token(jwt_secret, token) {
            Ok(claims) => {
                return Ok(ValidatedCookieSession {
                    claims,
                    candidate_count,
                    selected_index: index,
                    rejected_candidates,
                });
            }
            Err(error) => {
                let reason = classify_session_failure(&error);
                let signed_claims = (reason == SessionFailureReason::Expired)
                    .then(|| jwt::validate_expired_token_for_diagnostics(jwt_secret, token).ok())
                    .flatten();
                rejected_candidates.push(RejectedSessionCandidate {
                    index,
                    reason,
                    signed_claims,
                });
            }
        }
    }

    Err(CookieSessionError::Invalid {
        candidate_count,
        rejected_candidates,
    })
}

#[tracing::instrument(level = "debug", skip(session), fields(path))]
pub(super) fn trace_validated_session(path: &str, session: &ValidatedCookieSession) {
    if session.rejected_candidates.is_empty() {
        tracing::debug!(
            event = "auth.session_validated",
            path,
            username = %session.claims.sub,
            is_admin = session.claims.is_admin,
            issued_at = session.claims.iat,
            expires_at = session.claims.exp,
            cookie_candidate_count = session.candidate_count,
            selected_candidate_index = session.selected_index,
            "Session cookie validated"
        );
        return;
    }

    let rejected_reasons: Vec<&str> = session
        .rejected_candidates
        .iter()
        .map(|candidate| candidate.reason.as_str())
        .collect();
    let rejected_indices: Vec<usize> = session
        .rejected_candidates
        .iter()
        .map(|candidate| candidate.index)
        .collect();
    tracing::warn!(
        event = "auth.session_duplicate_recovered",
        path,
        username = %session.claims.sub,
        is_admin = session.claims.is_admin,
        issued_at = session.claims.iat,
        expires_at = session.claims.exp,
        cookie_candidate_count = session.candidate_count,
        selected_candidate_index = session.selected_index,
        rejected_candidate_indices = ?rejected_indices,
        rejected_reasons = ?rejected_reasons,
        "Recovered session using a valid duplicate cookie"
    );
}

#[tracing::instrument(level = "debug", skip(error), fields(path, action))]
pub(super) fn trace_rejected_session(path: &str, error: &CookieSessionError, action: &'static str) {
    match error {
        CookieSessionError::Missing => {
            tracing::debug!(
                event = "auth.session_missing",
                path,
                action,
                "No session cookie was supplied"
            );
        }
        CookieSessionError::Invalid {
            candidate_count,
            rejected_candidates,
        } => {
            let reasons: Vec<&str> = rejected_candidates
                .iter()
                .map(|candidate| candidate.reason.as_str())
                .collect();
            let signed_claims = rejected_candidates
                .iter()
                .find_map(|candidate| candidate.signed_claims.as_ref());
            let suspicious = rejected_candidates
                .iter()
                .any(|candidate| candidate.reason.is_suspicious());

            if suspicious {
                tracing::warn!(
                    event = "auth.session_rejected",
                    path,
                    action,
                    cookie_candidate_count = candidate_count,
                    validation_reasons = ?reasons,
                    "Rejected invalid session cookies"
                );
            } else if let Some(claims) = signed_claims {
                tracing::info!(
                    event = "auth.session_rejected",
                    path,
                    action,
                    cookie_candidate_count = candidate_count,
                    validation_reasons = ?reasons,
                    username = %claims.sub,
                    is_admin = claims.is_admin,
                    issued_at = claims.iat,
                    expires_at = claims.exp,
                    "Rejected expired session cookie"
                );
            } else {
                tracing::info!(
                    event = "auth.session_rejected",
                    path,
                    action,
                    cookie_candidate_count = candidate_count,
                    validation_reasons = ?reasons,
                    "Rejected invalid session cookies"
                );
            }

            tracing::info!(
                event = "auth.session_cleared",
                path,
                action,
                cookie_candidate_count = candidate_count,
                "Clearing invalid session cookie"
            );
        }
    }
}

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
    if let Ok(session) = validate_cookie_session(
        headers.get(header::COOKIE).and_then(|h| h.to_str().ok()),
        jwt_secret,
    ) {
        return Ok((session.claims.sub, session.claims.is_admin));
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
/// Missing sessions redirect with `session_required`; invalid sessions are cleared and redirect
/// with `session_expired`.
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

/// Custom rejection type that redirects to login.
#[derive(Debug, Clone, Copy)]
pub enum AuthRedirect {
    SessionRequired,
    InvalidSession,
}

impl IntoResponse for AuthRedirect {
    fn into_response(self) -> Response {
        match self {
            Self::SessionRequired => Redirect::to("/login?error=session_required").into_response(),
            Self::InvalidSession => (
                [(header::SET_COOKIE, cookies::create_clear_cookie())],
                Redirect::to("/login?error=session_expired"),
            )
                .into_response(),
        }
    }
}

fn redirect_for_session_error(error: &CookieSessionError) -> AuthRedirect {
    match error {
        CookieSessionError::Missing => AuthRedirect::SessionRequired,
        CookieSessionError::Invalid { .. } => AuthRedirect::InvalidSession,
    }
}

impl FromRequestParts<AppState> for AuthenticatedUserFromCookie {
    type Rejection = AuthRedirect;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let path = parts.uri.path();
        let session = validate_cookie_session(
            parts
                .headers
                .get(header::COOKIE)
                .and_then(|value| value.to_str().ok()),
            &state.jwt_secret,
        )
        .map_err(|error| {
            trace_rejected_session(path, &error, "redirect_to_login");
            redirect_for_session_error(&error)
        })?;
        trace_validated_session(path, &session);

        Ok(AuthenticatedUserFromCookie {
            username: session.claims.sub,
            is_admin: session.claims.is_admin,
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
        let path = parts.uri.path();
        let session = validate_cookie_session(
            parts
                .headers
                .get(header::COOKIE)
                .and_then(|value| value.to_str().ok()),
            &state.jwt_secret,
        )
        .map_err(|error| {
            trace_rejected_session(path, &error, "redirect_to_login");
            redirect_for_session_error(&error).into_response()
        })?;
        trace_validated_session(path, &session);
        let claims = session.claims;

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
    use jsonwebtoken::{EncodingKey, Header, encode};
    use shared::auth::jwt::Claims;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct CapturedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for CapturedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("trace buffer lock poisoned")
                .extend(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    const TEST_SECRET: &[u8] = b"test_secret_32_bytes_long_enough";

    fn make_token(username: &str, is_admin: bool) -> String {
        jwt::create_token(TEST_SECRET, username, is_admin, 3600)
            .expect("Failed to create test token")
    }

    fn make_expired_token() -> String {
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

    #[test]
    fn test_expired_cookie_before_valid_replacement_recovers() {
        let expired_token = make_expired_token();
        let valid_token = make_token("replacement_user", false);
        let cookie_header = format!("sherpa_auth={}; sherpa_auth={}", expired_token, valid_token);

        let session = validate_cookie_session(Some(&cookie_header), TEST_SECRET)
            .expect("valid replacement cookie should be accepted");

        assert_eq!(session.claims.sub, "replacement_user");
        assert_eq!(session.candidate_count, 2);
        assert_eq!(session.selected_index, 1);
        assert_eq!(session.rejected_candidates.len(), 1);
        assert_eq!(
            session.rejected_candidates[0].reason,
            SessionFailureReason::Expired
        );
    }

    #[test]
    fn test_valid_cookie_before_expired_duplicate_succeeds() {
        let valid_token = make_token("first_user", true);
        let expired_token = make_expired_token();
        let cookie_header = format!("sherpa_auth={}; sherpa_auth={}", valid_token, expired_token);

        let session = validate_cookie_session(Some(&cookie_header), TEST_SECRET)
            .expect("first valid cookie should be accepted");

        assert_eq!(session.claims.sub, "first_user");
        assert_eq!(session.candidate_count, 2);
        assert_eq!(session.selected_index, 0);
        assert!(session.rejected_candidates.is_empty());
    }

    #[test]
    fn test_invalid_session_redirect_clears_cookie() {
        let response = AuthRedirect::InvalidSession.into_response();

        assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/login?error=session_expired"
        );
        let clear_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("invalid session redirect should clear the cookie")
            .to_str()
            .expect("clear cookie should be a valid header");
        assert!(clear_cookie.contains("Max-Age=0"));
        assert!(clear_cookie.contains("Expires=Thu, 01 Jan 1970 00:00:00 GMT"));
    }

    #[test]
    fn test_missing_session_redirect_does_not_set_cookie() {
        let response = AuthRedirect::SessionRequired.into_response();

        assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/login?error=session_required"
        );
        assert!(!response.headers().contains_key(header::SET_COOKIE));
    }

    #[test]
    fn test_rejected_session_trace_does_not_include_raw_cookie() {
        const RAW_COOKIE: &str = "sherpa_auth=SUPER_SECRET_RAW_TOKEN";
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = CapturedWriter(Arc::clone(&output));
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        let error = validate_cookie_session(Some(RAW_COOKIE), TEST_SECRET)
            .expect_err("malformed session should be rejected");

        tracing::subscriber::with_default(subscriber, || {
            trace_rejected_session("/", &error, "redirect_to_login");
        });

        let output = output.lock().expect("trace buffer lock poisoned");
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("auth.session_rejected"));
        assert!(output.contains("malformed"));
        assert!(!output.contains("SUPER_SECRET_RAW_TOKEN"));
        assert!(!output.contains(RAW_COOKIE));
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
