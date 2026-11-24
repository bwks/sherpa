//! User management request and response data structures.
//!
//! These types define the JSON-RPC request/response formats for user management operations.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Request to create a new user (admin only).
///
/// It takes no SSH keys: a key on an account is installed on its labs' nodes, so only the user
/// adds their own. Unknown fields are refused, so a request that still sends `ssh_keys` fails
/// instead of creating the user without them.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub is_admin: bool,
    pub token: String,
}

/// Response after creating a user
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateUserResponse {
    pub success: bool,
    pub username: String,
    pub is_admin: bool,
}

/// Request to list all users (admin only)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListUsersRequest {
    pub token: String,
}

/// Response with list of users
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListUsersResponse {
    pub users: Vec<UserInfo>,
}

/// User information (safe for display, no password hash)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserInfo {
    pub username: String,
    pub is_admin: bool,
    pub ssh_keys: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub device_password_set: bool,
}

/// Request to delete a user (admin only)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteUserRequest {
    pub username: String,
    pub token: String,
}

/// Response after deleting a user
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteUserResponse {
    pub success: bool,
    pub username: String,
}

/// Request to unlock an account that failed logins locked (admin only)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UnlockUserRequest {
    pub username: String,
    pub token: String,
}

/// Response after unlocking an account
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UnlockUserResponse {
    pub success: bool,
    pub username: String,
}

/// Request to change user password
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangePasswordRequest {
    pub username: String,
    pub new_password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_password: Option<String>,
    pub token: String,
}

/// Response after changing password
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangePasswordResponse {
    pub username: String,
    pub success: bool,
}

/// Request to set your own device password, your login password on lab nodes. Only the
/// account's own user may set it, admins included, and it needs their Sherpa login password.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetDevicePasswordRequest {
    pub username: String,
    pub login_password: String,
    pub device_password: String,
    pub token: String,
}

impl fmt::Debug for SetDevicePasswordRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetDevicePasswordRequest")
            .field("username", &self.username)
            .field("login_password", &"[REDACTED]")
            .field("device_password", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

/// Response after setting a device password
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetDevicePasswordResponse {
    pub username: String,
    pub device_password_set: bool,
}

/// Request to get detailed user info
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetUserInfoRequest {
    pub username: String,
    pub token: String,
}

/// Response with detailed user information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetUserInfoResponse {
    pub user: UserInfo,
}

/// Request to add an SSH public key to a user
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddSshKeyRequest {
    pub username: String,
    pub ssh_key: String,
    pub token: String,
}

/// Request to remove an SSH public key from a user
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoveSshKeyRequest {
    pub username: String,
    pub ssh_key: String,
    pub token: String,
}

/// A user's SSH public keys after a change
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserSshKeysResponse {
    pub username: String,
    pub ssh_keys: Vec<String>,
}
