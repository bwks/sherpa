//! REST user operations.

use super::labs_api::require_admin_auth;
use axum::Json;
use axum::extract::{Path, State};

use crate::daemon::state::AppState;
use crate::services::{user_delete, user_password, user_ssh_keys};

use crate::api::errors::ApiError;
use crate::api::extractors::AuthenticatedUser;

use shared::data::{
    AddSshKeyRequest, ChangePasswordRequest, ChangePasswordResponse, CreateUserRequest,
    CreateUserResponse, DeleteUserResponse, GetUserInfoResponse, ListUsersResponse,
    RemoveSshKeyRequest, UserInfo, UserSshKeysResponse,
};
use shared::util::normalize_username;
use shared::validators::validate_username;

/// Create a user (admin only)
///
/// POST `/api/v1/admin/users`
pub async fn create_user_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Json(request): Json<CreateUserRequest>,
) -> Result<Json<CreateUserResponse>, ApiError> {
    require_admin_auth(&auth)?;

    let username = normalize_username(&request.username);
    validate_username(&username).map_err(|e| ApiError::bad_request(e.to_string()))?;

    let user = db::create_user(
        &state.db,
        username,
        &request.password,
        request.is_admin,
        request.ssh_keys,
    )
    .await
    .map_err(ApiError::from)?;

    Ok(Json(CreateUserResponse {
        success: true,
        username: user.username,
        is_admin: user.is_admin,
    }))
}

/// List users (admin only)
///
/// GET `/api/v1/admin/users`
pub async fn list_users_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
) -> Result<Json<ListUsersResponse>, ApiError> {
    require_admin_auth(&auth)?;

    let users = db::list_users(&state.db).await.map_err(ApiError::from)?;

    let user_list: Vec<UserInfo> = users
        .into_iter()
        .map(|u| UserInfo {
            username: u.username,
            is_admin: u.is_admin,
            ssh_keys: u.ssh_keys,
            created_at: u.created_at.as_second(),
            updated_at: u.updated_at.as_second(),
        })
        .collect();

    Ok(Json(ListUsersResponse { users: user_list }))
}

/// Delete a user (admin only)
///
/// DELETE `/api/v1/admin/users/{username}`
///
/// Refuses self-delete and the last admin (403), and users who own labs (409).
pub async fn delete_user_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<DeleteUserResponse>, ApiError> {
    let response = user_delete::delete_user(&auth.into_context(), &username, &state)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(response))
}

/// Change a user's password
///
/// POST `/api/v1/admin/users/{username}/password`
///
/// Changing your own password needs `current_password`; see `user_password::change_password`.
pub async fn change_password_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(payload): Json<ChangePasswordRequest>,
) -> Result<Json<ChangePasswordResponse>, ApiError> {
    let response = user_password::change_password(
        &auth.into_context(),
        &username,
        payload.current_password.as_deref(),
        &payload.new_password,
        &state.db,
    )
    .await
    .map_err(ApiError::from)?;
    Ok(Json(response))
}

/// Add an SSH public key to a user
///
/// POST `/api/v1/admin/users/{username}/ssh-keys`
pub async fn add_ssh_key_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(payload): Json<AddSshKeyRequest>,
) -> Result<Json<UserSshKeysResponse>, ApiError> {
    let response =
        user_ssh_keys::add_ssh_key(&auth.into_context(), &username, &payload.ssh_key, &state)
            .await
            .map_err(ApiError::from)?;
    Ok(Json(response))
}

/// Remove an SSH public key, given as the key or its SHA256 fingerprint, from a user
///
/// DELETE `/api/v1/admin/users/{username}/ssh-keys`
pub async fn remove_ssh_key_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(payload): Json<RemoveSshKeyRequest>,
) -> Result<Json<UserSshKeysResponse>, ApiError> {
    let response =
        user_ssh_keys::remove_ssh_key(&auth.into_context(), &username, &payload.ssh_key, &state)
            .await
            .map_err(ApiError::from)?;
    Ok(Json(response))
}

/// Get user info
///
/// GET `/api/v1/admin/users/{username}`
pub async fn get_user_info_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<GetUserInfoResponse>, ApiError> {
    // Admins can view any user; users can only view themselves
    if !auth.is_admin && auth.username != normalize_username(&username) {
        return Err(ApiError::forbidden("You can only view your own profile"));
    }

    let user = db::get_user(&state.db, &username)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(GetUserInfoResponse {
        user: UserInfo {
            username: user.username,
            is_admin: user.is_admin,
            ssh_keys: user.ssh_keys,
            created_at: user.created_at.as_second(),
            updated_at: user.updated_at.as_second(),
        },
    }))
}
