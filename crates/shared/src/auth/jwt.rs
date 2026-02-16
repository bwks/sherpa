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
    /// Subject - the username of the authenticated user
    pub sub: String,

    /// Expiration time (Unix timestamp in seconds)
    pub exp: i64,

    /// Issued at time (Unix timestamp in seconds)
    pub iat: i64,

    /// Whether the user has admin privileges
    pub is_admin: bool,
}

impl Claims {
    /// Create new JWT claims for a user.
    ///
    /// # Arguments
    /// * `username` - The username to encode in the token
    /// * `is_admin` - Whether the user has admin privileges
    /// * `expiry_seconds` - How long the token should be valid (typically 7 days = 604800 seconds)
    ///
    /// # Returns
    /// A new `Claims` instance with the current time as `iat` and calculated `exp`
    pub fn new(username: String, is_admin: bool, expiry_seconds: i64) -> Self {
        let now = jiff::Timestamp::now().as_second();
        Self {
            sub: username,
            exp: now + expiry_seconds,
            iat: now,
            is_admin,
        }
    }

    /// Check if the token has expired.
    ///
    /// # Returns
    /// `true` if the current time is past the expiration time
    pub fn is_expired(&self) -> bool {
        jiff::Timestamp::now().as_second() > self.exp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claims_new() {
        let username = "testuser".to_string();
        let expiry = 3600; // 1 hour

        let claims = Claims::new(username.clone(), false, expiry);

        assert_eq!(claims.sub, username);
        assert_eq!(claims.is_admin, false);
        assert!(claims.exp > claims.iat);
        assert_eq!(claims.exp - claims.iat, expiry);
    }

    #[test]
    fn test_claims_new_admin() {
        let claims = Claims::new("admin".to_string(), true, 3600);
        assert_eq!(claims.is_admin, true);
    }

    #[test]
    fn test_claims_not_expired() {
        let claims = Claims::new("user".to_string(), false, 3600);
        assert_eq!(claims.is_expired(), false);
    }

    #[test]
    fn test_claims_expired() {
        let now = jiff::Timestamp::now().as_second();
        let claims = Claims {
            sub: "user".to_string(),
            exp: now - 100, // expired 100 seconds ago
            iat: now - 3700,
            is_admin: false,
        };
        assert_eq!(claims.is_expired(), true);
    }
}
