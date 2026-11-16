//! Server administration RPC methods.

use serde_json::Value;

use super::dispatch::respond;
use crate::api::websocket::messages::RpcError;
use crate::auth::context::AuthContext;
use crate::daemon::state::AppState;
use crate::services::backup;

const RPC_MSG_BACKUP_FAILED: &str = "Database backup failed";

pub(super) async fn backup(ctx: &AuthContext, state: &AppState) -> Result<Value, RpcError> {
    respond(
        backup::backup_database(ctx, state).await,
        RPC_MSG_BACKUP_FAILED,
    )
}
