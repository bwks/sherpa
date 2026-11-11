//! The notice the login page shows, from the `error` code a redirect put in its query.

use std::fmt;

/// Why the login page was shown, from its `error` query parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginError {
    /// A page needed a session and the request had none.
    SessionRequired,
    /// The request's session was invalid or expired.
    SessionExpired,
    /// The user logged out.
    LogoutSuccess,
    /// A code this build does not know, shown as it came.
    Other(String),
}

impl LoginError {
    /// The notice for `code`; none for an empty one.
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "" => None,
            "session_required" => Some(Self::SessionRequired),
            "session_expired" => Some(Self::SessionExpired),
            "logout_success" => Some(Self::LogoutSuccess),
            other => Some(Self::Other(other.to_string())),
        }
    }
}

impl fmt::Display for LoginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionRequired => write!(f, "Please sign in to continue"),
            Self::SessionExpired => write!(f, "Your session has expired. Please sign in again."),
            Self::LogoutSuccess => write!(f, "You have been logged out successfully"),
            Self::Other(code) => write!(f, "{code}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_code_reads_as_its_notice() {
        for (code, error, text) in [
            (
                "session_required",
                LoginError::SessionRequired,
                "Please sign in to continue",
            ),
            (
                "session_expired",
                LoginError::SessionExpired,
                "Your session has expired. Please sign in again.",
            ),
            (
                "logout_success",
                LoginError::LogoutSuccess,
                "You have been logged out successfully",
            ),
            (
                "bad_thing",
                LoginError::Other("bad_thing".to_string()),
                "bad_thing",
            ),
        ] {
            assert_eq!(LoginError::from_code(code), Some(error.clone()), "{code}");
            assert_eq!(error.to_string(), text, "{code}");
        }
    }

    #[test]
    fn an_empty_code_shows_no_notice() {
        assert_eq!(LoginError::from_code(""), None);
    }
}
