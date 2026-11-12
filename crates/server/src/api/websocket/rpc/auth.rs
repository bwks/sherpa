//! Login, token validation and logout.

use serde_json::Value;

use shared::data;
use shared::error::RpcErrorCode;
use shared::konst::{
    JWT_TOKEN_EXPIRY_SECONDS, RPC_MSG_AUTH_ERROR, RPC_MSG_AUTH_INVALID,
    RPC_MSG_INVALID_PARAMS_LOGIN, RPC_MSG_INVALID_PARAMS_TOKEN, RPC_MSG_LOGOUT_FAILED,
    RPC_MSG_TOKEN_CREATE_FAILED,
};

use super::dispatch::{parse, respond};
use crate::api::websocket::messages::RpcError;
use crate::auth::context::AuthContext;
use crate::auth::{login, session};
use crate::daemon::state::AppState;

pub(super) async fn login(params: Value, state: &AppState) -> Result<Value, RpcError> {
    let request: data::LoginRequest = parse(params, RPC_MSG_INVALID_PARAMS_LOGIN)?;
    let user = match login::check_credentials(&state.db, &request.username, &request.password).await
    {
        Ok(Some(user)) => user,
        Ok(None) => {
            tracing::warn!(
                username = %request.username,
                "Login failed: unknown user or wrong password"
            );
            return Err(rpc_error(RpcErrorCode::AuthInvalid, RPC_MSG_AUTH_INVALID));
        }
        Err(error) => {
            tracing::error!(error = ?error, "Login check failed");
            return Err(rpc_error(RpcErrorCode::InternalError, RPC_MSG_AUTH_ERROR));
        }
    };
    let issued = session::start(
        &state.db,
        &state.jwt_secret,
        &user,
        data::SessionChannel::Cli,
        JWT_TOKEN_EXPIRY_SECONDS,
    )
    .await
    .map_err(|error| {
        tracing::error!(error = format!("{error:#}"), "Failed to start a session");
        rpc_error(RpcErrorCode::InternalError, RPC_MSG_TOKEN_CREATE_FAILED)
    })?;
    tracing::info!(
        username = %user.username,
        is_admin = user.is_admin,
        "User logged in successfully"
    );
    respond(
        Ok(data::LoginResponse {
            token: issued.token,
            username: user.username,
            is_admin: user.is_admin,
            expires_at: issued.expires_at,
        }),
        RPC_MSG_AUTH_ERROR,
    )
}

pub(super) async fn validate(params: Value, state: &AppState) -> Result<Value, RpcError> {
    let request: data::ValidateRequest = parse(params, RPC_MSG_INVALID_PARAMS_TOKEN)?;
    respond(
        session::token_status(&state.db, &state.jwt_secret, &request.token).await,
        RPC_MSG_AUTH_ERROR,
    )
}

/// End the caller's session: the one the request's token belongs to.
pub(super) async fn logout(ctx: &AuthContext, state: &AppState) -> Result<Value, RpcError> {
    respond(
        session::logout(&state.db, &ctx.session_id)
            .await
            .map(|ended| data::LogoutResponse { ended }),
        RPC_MSG_LOGOUT_FAILED,
    )
}

fn rpc_error(code: RpcErrorCode, message: &str) -> RpcError {
    RpcError {
        code,
        message: message.to_string(),
        context: None,
        data: None,
    }
}
