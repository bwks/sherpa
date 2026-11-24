//! Authentication request and response data structures.
//!
//! These types define the JSON-RPC request/response formats for authentication operations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Request to authenticate a user and receive a JWT token
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Response from a successful login
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
    pub is_admin: bool,
    pub expires_at: i64,
}

/// Request to validate a JWT token
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ValidateRequest {
    pub token: String,
}

/// Response from logout
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LogoutResponse {
    pub ended: bool,
}

/// Response from token validation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ValidateResponse {
    pub valid: bool,
    pub username: Option<String>,
    pub is_admin: Option<bool>,
    pub expires_at: Option<i64>,
}
