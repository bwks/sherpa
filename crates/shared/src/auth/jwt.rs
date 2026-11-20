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
