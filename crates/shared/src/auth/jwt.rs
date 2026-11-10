//! JWT token claims structure.
//!
//! This module defines the JWT claims used for authentication tokens.
//! The claims include user identification, expiration, and admin status.

use serde::{Deserialize, Serialize};

/// JWT claims structure for authentication tokens.
///
/// Contains the standard JWT claims plus custom fields for user identification
/// and authorization (admin status).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Claims {
    /// Subject - the username of the authenticated user, for display. The server takes the
    /// user from the session, not from this claim.
    pub sub: String,

    /// The session this token belongs to. The server accepts the token only while the session
    /// exists. Tokens from before sessions were added have no `sid` and are rejected.
    pub sid: String,

    /// Expiration time (Unix timestamp in seconds)
    pub exp: i64,

    /// Issued at time (Unix timestamp in seconds)
    pub iat: i64,

    /// Whether the user has admin privileges
    pub is_admin: bool,
}

impl Claims {
    /// Create JWT claims for a token of session `session_id`.
    ///
    /// # Arguments
    /// * `username` - The username to encode in the token
    /// * `session_id` - The session the token belongs to
    /// * `is_admin` - Whether the user has admin privileges
    /// * `issued_at` - When the token is issued, in Unix seconds
    /// * `expires_at` - When the token and its session expire, in Unix seconds
    #[must_use]
    pub fn new(
        username: String,
        session_id: String,
        is_admin: bool,
        issued_at: i64,
        expires_at: i64,
    ) -> Self {
        Self {
            sub: username,
            sid: session_id,
            exp: expires_at,
            iat: issued_at,
            is_admin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claims_new() {
        let claims = Claims::new("testuser".to_owned(), "sid1".to_owned(), false, 100, 3700);

        assert_eq!(claims.sub, "testuser");
        assert_eq!(claims.sid, "sid1");
        assert!(!claims.is_admin);
        assert_eq!(claims.iat, 100);
        assert_eq!(claims.exp, 3700);
    }

    #[test]
    fn test_claims_new_admin() {
        let claims = Claims::new("admin".to_owned(), "sid1".to_owned(), true, 0, 3600);
        assert!(claims.is_admin);
    }

    #[test]
    fn test_claims_serde_roundtrip() {
        let claims = Claims::new("testuser".to_owned(), "sid1".to_owned(), true, 0, 604_800);
        let json = serde_json::to_string(&claims).expect("serializes");
        let deserialized: Claims = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(claims, deserialized);
    }

    #[test]
    fn claims_without_a_session_are_rejected() {
        let old = r#"{"sub":"alice","exp":2000000000,"iat":1000000000,"is_admin":false}"#;
        assert!(serde_json::from_str::<Claims>(old).is_err());
    }
}
