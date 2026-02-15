//! User management request and response data structures.
//!
//! These types define the JSON-RPC request/response formats for user management operations.

use serde::{Deserialize, Serialize};

/// Request to create a new user (admin only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserRequest {
    /// Username for the new user
    pub username: String,
    /// Password for the new user (will be hashed server-side)
    pub password: String,
    /// Whether the user should have admin privileges
    pub is_admin: bool,
    /// Optional SSH public keys
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_keys: Option<Vec<String>>,
    /// Caller's authentication token
    pub token: String,
}

/// Response after creating a user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserResponse {
    /// Whether the creation was successful
    pub success: bool,
    /// Username of the created user
    pub username: String,
    /// Whether the user has admin privileges
    pub is_admin: bool,
}

/// Request to list all users (admin only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListUsersRequest {
    /// Caller's authentication token
    pub token: String,
}

/// Response with list of users
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListUsersResponse {
    /// List of users (without sensitive data)
    pub users: Vec<UserInfo>,
}

/// User information (safe for display, no password hash)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    /// Username
    pub username: String,
    /// Whether the user has admin privileges
    pub is_admin: bool,
    /// SSH public keys
    pub ssh_keys: Vec<String>,
    /// When the user was created (Unix timestamp)
    pub created_at: i64,
    /// When the user was last updated (Unix timestamp)
    pub updated_at: i64,
}

/// Request to delete a user (admin only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteUserRequest {
    /// Username to delete
    pub username: String,
    /// Caller's authentication token
    pub token: String,
}

/// Response after deleting a user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteUserResponse {
    /// Whether the deletion was successful
    pub success: bool,
    /// Username of the deleted user
    pub username: String,
}

/// Request to change user password
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    /// User whose password to change
    pub username: String,
    /// New password (will be hashed server-side)
    pub new_password: String,
    /// Caller's authentication token
    pub token: String,
}

/// Response after changing password
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePasswordResponse {
    /// Username whose password was changed
    pub username: String,
    /// Whether the change was successful
    pub success: bool,
}

/// Request to get detailed user info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetUserInfoRequest {
    /// Username to get info for
    pub username: String,
    /// Caller's authentication token
    pub token: String,
}

/// Response with detailed user information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetUserInfoResponse {
    /// User information
    pub user: UserInfo,
}
