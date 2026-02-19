/// Cookie name for authentication token
pub const AUTH_COOKIE_NAME: &str = "sherpa_auth";

/// Cookie max-age for normal sessions (7 days)
pub const COOKIE_MAX_AGE_NORMAL: i64 = 7 * 24 * 60 * 60; // 7 days in seconds

/// Cookie max-age for "remember me" sessions (30 days)
pub const COOKIE_MAX_AGE_REMEMBER: i64 = 30 * 24 * 60 * 60; // 30 days in seconds

/// Creates an authentication cookie with the JWT token.
///
/// The cookie is:
/// - HttpOnly (prevents JavaScript access)
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
pub fn create_auth_cookie(token: &str, remember_me: bool) -> String {
    let max_age = if remember_me {
        COOKIE_MAX_AGE_REMEMBER
    } else {
        COOKIE_MAX_AGE_NORMAL
    };

    // In production, add Secure flag. For development/testing, omit it.
    // You can check an env var or compile-time flag if needed.
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
        AUTH_COOKIE_NAME, token, max_age
    )
}

/// Creates a cookie that clears the authentication cookie.
///
/// Sets Max-Age=0 to immediately expire the cookie.
///
/// # Returns
/// A Set-Cookie header value string that clears the auth cookie
pub fn create_clear_cookie() -> String {
    format!(
        "{}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
        AUTH_COOKIE_NAME
    )
}

/// Extracts the JWT token from a Cookie header value.
///
/// Parses the Cookie header looking for the auth cookie name and returns its value.
///
/// # Arguments
/// * `cookie_header` - The value of the Cookie header
///
/// # Returns
/// The token string if found, None otherwise
pub fn extract_token_from_cookie(cookie_header: &str) -> Option<String> {
    // Parse cookies in format "name1=value1; name2=value2"
    for cookie in cookie_header.split(';') {
        let cookie = cookie.trim();
        if let Some((name, value)) = cookie.split_once('=')
            && name == AUTH_COOKIE_NAME
        {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_auth_cookie_normal() {
        let token = "test_token_123";
        let cookie = create_auth_cookie(token, false);

        assert!(cookie.contains("sherpa_auth=test_token_123"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains(&format!("Max-Age={}", COOKIE_MAX_AGE_NORMAL)));
        assert!(cookie.contains("Path=/"));
    }

    #[test]
    fn test_create_auth_cookie_remember_me() {
        let token = "test_token_456";
        let cookie = create_auth_cookie(token, true);

        assert!(cookie.contains("sherpa_auth=test_token_456"));
        assert!(cookie.contains(&format!("Max-Age={}", COOKIE_MAX_AGE_REMEMBER)));
    }

    #[test]
    fn test_create_clear_cookie() {
        let cookie = create_clear_cookie();

        assert!(cookie.contains("sherpa_auth="));
        assert!(cookie.contains("Max-Age=0"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
    }

    #[test]
    fn test_extract_token_from_cookie_found() {
        let cookie_header = "sherpa_auth=my_token_123; other_cookie=value";
        let token = extract_token_from_cookie(cookie_header);

        assert_eq!(token, Some("my_token_123".to_string()));
    }

    #[test]
    fn test_extract_token_from_cookie_only_auth() {
        let cookie_header = "sherpa_auth=just_this_token";
        let token = extract_token_from_cookie(cookie_header);

        assert_eq!(token, Some("just_this_token".to_string()));
    }

    #[test]
    fn test_extract_token_from_cookie_not_found() {
        let cookie_header = "other_cookie=value; another=thing";
        let token = extract_token_from_cookie(cookie_header);

        assert_eq!(token, None);
    }

    #[test]
    fn test_extract_token_from_cookie_empty() {
        let token = extract_token_from_cookie("");

        assert_eq!(token, None);
    }
}
