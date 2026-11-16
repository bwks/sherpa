//! REST user operations.

use super::labs_api::require_admin_auth;
use axum::Json;
use axum::extract::{Path, State};

use crate::daemon::state::AppState;
use crate::services::{user_delete, user_device_password, user_password, user_ssh_keys};

use crate::api::errors::ApiError;
use crate::api::extractors::AuthenticatedUser;

use shared::data::{
    AddSshKeyRequest, ChangePasswordRequest, ChangePasswordResponse, CreateUserRequest,
    CreateUserResponse, DeleteUserResponse, GetUserInfoResponse, ListUsersResponse,
    RemoveSshKeyRequest, SetDevicePasswordRequest, SetDevicePasswordResponse, UserSshKeysResponse,
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

    let users = user_device_password::list_user_info(&state.db)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(ListUsersResponse { users }))
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

/// Set your own device password; self only, admins included
///
/// POST `/api/v1/admin/users/{username}/device-password`
pub async fn set_device_password_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(payload): Json<SetDevicePasswordRequest>,
) -> Result<Json<SetDevicePasswordResponse>, ApiError> {
    let response = user_device_password::set_device_password(
        &auth.into_context(),
        &username,
        &payload.login_password,
        &payload.device_password,
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

    let user = user_device_password::user_info(user, &state.db)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(GetUserInfoResponse { user }))
}
