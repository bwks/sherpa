//! Login and token validation, the RPC methods that take no token.

use serde_json::Value;

use shared::data;
use shared::error::RpcErrorCode;
use shared::konst::{
    JWT_TOKEN_EXPIRY_SECONDS, RPC_MSG_AUTH_ERROR, RPC_MSG_AUTH_INVALID,
    RPC_MSG_INVALID_PARAMS_LOGIN, RPC_MSG_INVALID_PARAMS_TOKEN, RPC_MSG_TOKEN_CREATE_FAILED,
};

use super::dispatch::{parse, respond};
use crate::api::websocket::messages::RpcError;
use crate::auth::{jwt, login};
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
    let token = jwt::create_token(
        &state.jwt_secret,
        &user.username,
        user.is_admin,
        JWT_TOKEN_EXPIRY_SECONDS,
    )
    .map_err(|error| {
        tracing::error!(%error, "Failed to create JWT token");
        rpc_error(RpcErrorCode::InternalError, RPC_MSG_TOKEN_CREATE_FAILED)
    })?;
    tracing::info!(
        username = %user.username,
        is_admin = user.is_admin,
        "User logged in successfully"
    );
    respond(
        Ok(data::LoginResponse {
            token,
            username: user.username,
            is_admin: user.is_admin,
            expires_at: jiff::Timestamp::now().as_second() + JWT_TOKEN_EXPIRY_SECONDS,
        }),
        RPC_MSG_AUTH_ERROR,
    )
}

pub(super) fn validate(params: Value, state: &AppState) -> Result<Value, RpcError> {
    let request: data::ValidateRequest = parse(params, RPC_MSG_INVALID_PARAMS_TOKEN)?;
    respond(
        Ok(jwt::token_status(&state.jwt_secret, &request.token)),
        RPC_MSG_AUTH_ERROR,
    )
}

fn rpc_error(code: RpcErrorCode, message: &str) -> RpcError {
    RpcError {
        code,
        message: message.to_string(),
        context: None,
    }
}
