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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_login_request_serde_roundtrip() {
        let req = LoginRequest {
            username: "admin".to_string(),
            password: "secret".to_string(),
        };
        let json = serde_json::to_string(&req).expect("serializes");
        let back: LoginRequest = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.username, "admin");
        assert_eq!(back.password, "secret");
    }

    #[test]
    fn test_login_response_serde_roundtrip() {
        let resp = LoginResponse {
            token: "jwt.token.here".to_string(),
            username: "admin".to_string(),
            is_admin: true,
            expires_at: 1700000000,
        };
        let json = serde_json::to_string(&resp).expect("serializes");
        let back: LoginResponse = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.token, "jwt.token.here");
        assert_eq!(back.is_admin, true);
        assert_eq!(back.expires_at, 1700000000);
    }

    #[test]
    fn test_validate_response_serde_roundtrip() {
        let resp = ValidateResponse {
            valid: true,
            username: Some("admin".to_string()),
            is_admin: Some(true),
            expires_at: Some(1700000000),
        };
        let json = serde_json::to_string(&resp).expect("serializes");
        let back: ValidateResponse = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.valid, true);
        assert_eq!(back.username, Some("admin".to_string()));
    }

    #[test]
    fn test_validate_response_invalid_token() {
        let resp = ValidateResponse {
            valid: false,
            username: None,
            is_admin: None,
            expires_at: None,
        };
        let json = serde_json::to_string(&resp).expect("serializes");
        let back: ValidateResponse = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.valid, false);
        assert!(back.username.is_none());
    }
}
