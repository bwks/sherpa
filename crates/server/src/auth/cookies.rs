/// Cookie name for authentication token
pub const AUTH_COOKIE_NAME: &str = "sherpa_auth";

/// Cookie max-age for normal sessions (7 days)
pub const COOKIE_MAX_AGE_NORMAL: i64 = 7 * 24 * 60 * 60; // 7 days in seconds

/// Cookie max-age for "remember me" sessions (30 days)
pub const COOKIE_MAX_AGE_REMEMBER: i64 = 30 * 24 * 60 * 60; // 30 days in seconds

/// Creates an authentication cookie with the JWT token.
///
/// The cookie is:
/// - `HttpOnly` (prevents JavaScript access)
/// - Secure (HTTPS only in production)
/// - SameSite=Strict (CSRF protection)
/// - Path=/ (available for entire site)
///
/// # Arguments
/// * `token` - JWT token string
/// * `remember_me` - If true, extends cookie lifetime to 30 days instead of 7
///
/// # Returns
/// A Set-Cookie header value string
#[must_use]
pub fn create_auth_cookie(token: &str, remember_me: bool) -> String {
    let max_age = if remember_me {
        COOKIE_MAX_AGE_REMEMBER
    } else {
        COOKIE_MAX_AGE_NORMAL
    };

    // In production, add Secure flag. For development/testing, omit it.
    // You can check an env var or compile-time flag if needed.
    format!(
        "{AUTH_COOKIE_NAME}={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={max_age}"
    )
}

/// Creates a cookie that clears the authentication cookie.
///
/// Sets Max-Age=0 to immediately expire the cookie.
///
/// # Returns
/// A Set-Cookie header value string that clears the auth cookie
#[must_use]
pub fn create_clear_cookie() -> String {
    format!(
        "{AUTH_COOKIE_NAME}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT"
    )
}

/// Extract every authentication token from a Cookie header.
#[tracing::instrument(level = "debug", skip(cookie_header))]
pub(crate) fn extract_tokens_from_cookie(cookie_header: &str) -> Vec<&str> {
    cookie_header
        .split(';')
        .filter_map(|cookie| {
            let (name, value) = cookie.trim().split_once('=')?;
            (name == AUTH_COOKIE_NAME).then_some(value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_login_008_the_session_cookie_is_http_only_secure_strict_for_7_or_30_days() {
        let cookie = create_auth_cookie("test_token_123", false);

        assert!(cookie.contains("sherpa_auth=test_token_123"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Max-Age=604800"));
        assert!(cookie.contains("Path=/"));

        let remembered = create_auth_cookie("test_token_456", true);

        assert!(remembered.contains("sherpa_auth=test_token_456"));
        assert!(remembered.contains("HttpOnly"));
        assert!(remembered.contains("Secure"));
        assert!(remembered.contains("SameSite=Strict"));
        assert!(remembered.contains("Max-Age=2592000"));
    }

    #[test]
    fn test_create_clear_cookie() {
        let cookie = create_clear_cookie();

        assert!(cookie.contains("sherpa_auth="));
        assert!(cookie.contains("Max-Age=0"));
        assert!(cookie.contains("Expires=Thu, 01 Jan 1970 00:00:00 GMT"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
    }
}
