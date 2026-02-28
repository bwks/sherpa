//! Authentication request and response data structures.
//!
//! These types define the JSON-RPC request/response formats for authentication operations.

use serde::{Deserialize, Serialize};

/// Request to authenticate a user and receive a JWT token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    /// Username to authenticate
    pub username: String,
    /// Password for authentication
    pub password: String,
}

/// Response from a successful login
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    /// JWT token for authenticated requests
    pub token: String,
    /// Username of the authenticated user
    pub username: String,
    /// Whether the user has admin privileges
    pub is_admin: bool,
    /// Token expiration timestamp (Unix seconds)
    pub expires_at: i64,
}

/// Request to validate a JWT token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateRequest {
    /// JWT token to validate
    pub token: String,
}

/// Response from token validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateResponse {
    /// Whether the token is valid
    pub valid: bool,
    /// Username extracted from the token (if valid)
    pub username: Option<String>,
    /// Whether the user has admin privileges (if valid)
    pub is_admin: Option<bool>,
    /// Token expiration timestamp (if valid)
    pub expires_at: Option<i64>,
}
