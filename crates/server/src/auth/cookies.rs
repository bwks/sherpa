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
/// - `Secure` (sent over HTTPS only)
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

    /// The cookie's name and value, and its attributes in order.
    fn parse_cookie(cookie: &str) -> ((&str, &str), Vec<&str>) {
        let mut parts = cookie.split("; ");
        let pair = parts.next().unwrap().split_once('=').unwrap();
        (pair, parts.collect())
    }

    #[test]
    fn user_login_008_the_session_cookie_is_http_only_secure_strict_for_7_or_30_days() {
        for (remember_me, max_age) in [(false, "Max-Age=604800"), (true, "Max-Age=2592000")] {
            let cookie = create_auth_cookie("test_token_123", remember_me);
            assert_eq!(
                parse_cookie(&cookie),
                (
                    ("sherpa_auth", "test_token_123"),
                    vec!["Path=/", "HttpOnly", "Secure", "SameSite=Strict", max_age]
                )
            );
        }
    }

    #[test]
    fn user_sessions_026_the_cookie_that_ends_a_session_is_secure_http_only_strict_and_expired() {
        assert_eq!(
            parse_cookie(&create_clear_cookie()),
            (
                ("sherpa_auth", ""),
                vec![
                    "Path=/",
                    "HttpOnly",
                    "Secure",
                    "SameSite=Strict",
                    "Max-Age=0",
                    "Expires=Thu, 01 Jan 1970 00:00:00 GMT"
                ]
            )
        );
    }
}
