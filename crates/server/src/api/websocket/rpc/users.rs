//! User account RPC methods.

use anyhow::Context;
use serde_json::Value;

use shared::data;
use shared::error::RpcErrorCode;
use shared::konst::{
    RPC_MSG_ACCESS_DENIED_OWN_INFO, RPC_MSG_INVALID_PARAMS_ADD_SSH_KEY,
    RPC_MSG_INVALID_PARAMS_CHANGE_PASSWORD, RPC_MSG_INVALID_PARAMS_CREATE_USER,
    RPC_MSG_INVALID_PARAMS_DELETE_USER, RPC_MSG_INVALID_PARAMS_GET_USER_INFO,
    RPC_MSG_INVALID_PARAMS_REMOVE_SSH_KEY, RPC_MSG_SSH_KEY_ADD_FAILED,
    RPC_MSG_SSH_KEY_REMOVE_FAILED, RPC_MSG_USER_CREATE_FAILED, RPC_MSG_USER_DELETE_FAILED,
    RPC_MSG_USER_LIST_FAILED, RPC_MSG_USER_PASSWORD_UPDATE_FAILED,
};
use shared::util::normalize_username;

use super::dispatch::{parse, respond};
use crate::api::websocket::messages::RpcError;
use crate::auth::context::AuthContext;
use crate::daemon::state::AppState;
use crate::services::{user_delete, user_password, user_ssh_keys};

const RPC_MSG_USER_INFO_FAILED: &str = "Failed to get user info";

/// Admin only; the dispatcher checks.
pub(super) async fn create(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let request: data::CreateUserRequest = parse(params, RPC_MSG_INVALID_PARAMS_CREATE_USER)?;
    let created = db::create_user(
        &state.db,
        request.username,
        &request.password,
        request.is_admin,
        request.ssh_keys,
    )
    .await
    .map(|user| {
        tracing::info!(
            admin = %ctx.username,
            new_user = %user.username,
            is_admin = user.is_admin,
            "User created"
        );
        data::CreateUserResponse {
            success: true,
            username: user.username,
            is_admin: user.is_admin,
        }
    });
    respond(created, RPC_MSG_USER_CREATE_FAILED)
}

/// Admin only; the dispatcher checks.
pub(super) async fn list(ctx: &AuthContext, state: &AppState) -> Result<Value, RpcError> {
    let users = db::list_users(&state.db).await.map(|users| {
        tracing::debug!(admin = %ctx.username, count = users.len(), "Listed users");
        data::ListUsersResponse {
            users: users.into_iter().map(user_info).collect(),
        }
    });
    respond(users, RPC_MSG_USER_LIST_FAILED)
}

pub(super) async fn delete(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let request: data::DeleteUserRequest = parse(params, RPC_MSG_INVALID_PARAMS_DELETE_USER)?;
    respond(
        user_delete::delete_user(ctx, &request.username, state).await,
        RPC_MSG_USER_DELETE_FAILED,
    )
}

pub(super) async fn change_password(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let request: data::ChangePasswordRequest =
        parse(params, RPC_MSG_INVALID_PARAMS_CHANGE_PASSWORD)?;
    respond(
        user_password::change_password(
            ctx,
            &request.username,
            request.current_password.as_deref(),
            &request.new_password,
            &state.db,
        )
        .await,
        RPC_MSG_USER_PASSWORD_UPDATE_FAILED,
    )
}

/// Users see their own account; admins see anyone's.
pub(super) async fn info(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let request: data::GetUserInfoRequest = parse(params, RPC_MSG_INVALID_PARAMS_GET_USER_INFO)?;
    if !ctx.is_admin && normalize_username(&request.username) != ctx.username {
        tracing::warn!(
            user = %ctx.username,
            target = %request.username,
            "User attempted to view another user's info"
        );
        return Err(RpcError {
            code: RpcErrorCode::AccessDenied,
            message: RPC_MSG_ACCESS_DENIED_OWN_INFO.to_string(),
            context: None,
        });
    }
    let user = db::find_user(&state.db, &request.username)
        .await
        .context("Failed to look up user");
    match user {
        Ok(Some(user)) => respond(
            Ok(data::GetUserInfoResponse {
                user: user_info(user),
            }),
            RPC_MSG_USER_INFO_FAILED,
        ),
        Ok(None) => Err(RpcError {
            code: RpcErrorCode::NotFound,
            message: format!("User not found: {}", request.username),
            context: None,
        }),
        Err(error) => respond::<data::GetUserInfoResponse>(Err(error), RPC_MSG_USER_INFO_FAILED),
    }
}

pub(super) async fn add_ssh_key(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let request: data::AddSshKeyRequest = parse(params, RPC_MSG_INVALID_PARAMS_ADD_SSH_KEY)?;
    respond(
        user_ssh_keys::add_ssh_key(ctx, &request.username, &request.ssh_key, state).await,
        RPC_MSG_SSH_KEY_ADD_FAILED,
    )
}

pub(super) async fn remove_ssh_key(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let request: data::RemoveSshKeyRequest = parse(params, RPC_MSG_INVALID_PARAMS_REMOVE_SSH_KEY)?;
    respond(
        user_ssh_keys::remove_ssh_key(ctx, &request.username, &request.ssh_key, state).await,
        RPC_MSG_SSH_KEY_REMOVE_FAILED,
    )
}

/// The account details a user may see; no password hash.
fn user_info(user: data::DbUser) -> data::UserInfo {
    data::UserInfo {
        username: user.username,
        is_admin: user.is_admin,
        ssh_keys: user.ssh_keys,
        created_at: user.created_at.as_second(),
        updated_at: user.updated_at.as_second(),
    }
}
