//! The notice the login page shows, from the `error` code a redirect put in its query.

use std::fmt;

/// Why the login page was shown, from its `error` query parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginError {
    SessionRequired,
    SessionExpired,
    LogoutSuccess,
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
