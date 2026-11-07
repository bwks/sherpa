//! Server administration RPC methods.

use serde_json::Value;

use shared::konst::RPC_MSG_BACKUP_FAILED;

use super::dispatch::respond;
use crate::api::websocket::messages::RpcError;
use crate::auth::context::AuthContext;
use crate::daemon::state::AppState;
use crate::services::backup;

pub(super) async fn backup(ctx: &AuthContext, state: &AppState) -> Result<Value, RpcError> {
    respond(
        backup::backup_database(ctx, state).await,
        RPC_MSG_BACKUP_FAILED,
    )
}
