//! Lab and node RPC methods.

use serde::Deserialize;
use serde_json::Value;

use shared::data::{self, TailnetAuthKey};
use shared::konst::{
    RPC_MSG_IMPAIRMENT_UPDATE_FAILED, RPC_MSG_INVALID_PARAMS_IMPAIRMENT,
    RPC_MSG_INVALID_PARAMS_LAB_ID, RPC_MSG_INVALID_PARAMS_MANIFEST,
    RPC_MSG_INVALID_PARAMS_NODE_HOST_KEYS, RPC_MSG_INVALID_PARAMS_NODE_SECRETS,
    RPC_MSG_INVALID_PARAMS_REDEPLOY, RPC_MSG_INVALID_TAILSCALE_KEY, RPC_MSG_LAB_CANCEL_FAILED,
    RPC_MSG_LAB_CLEAN_FAILED, RPC_MSG_LAB_DESTROY_FAILED, RPC_MSG_LAB_DOWN_FAILED,
    RPC_MSG_LAB_INSPECT_FAILED, RPC_MSG_LAB_RESUME_FAILED, RPC_MSG_LAB_UP_FAILED,
    RPC_MSG_NODE_HOST_KEYS_FAILED, RPC_MSG_NODE_SECRETS_FAILED, RPC_MSG_REDEPLOY_FAILED,
};

use super::dispatch::{parse, parse_quietly, respond};
use crate::api::websocket::messages::RpcError;
use crate::auth::context::AuthContext;
use crate::daemon::state::AppState;
use crate::services::progress::ProgressSender;
use crate::services::{
    cancel, clean, destroy, down, download, host_keys, impairment, inspect, list_labs,
    node_secrets, redeploy, resume, up,
};

const RPC_MSG_DOWNLOAD_FAILED: &str = "Failed to download lab files";
const RPC_MSG_LABS_LIST_FAILED: &str = "Failed to list labs";

/// A lab, and optionally one of its nodes.
#[derive(Deserialize)]
struct LabParams {
    lab_id: String,
    #[serde(default)]
    node_name: Option<String>,
}

/// The Tailscale credential `resume` may carry.
#[derive(Deserialize)]
struct TailscaleParams {
    #[serde(default)]
    tailscale_auth_key: Option<TailnetAuthKey>,
}

fn lab_params(params: Value) -> Result<LabParams, RpcError> {
    parse(params, RPC_MSG_INVALID_PARAMS_LAB_ID)
}

pub(super) async fn list(ctx: &AuthContext, state: &AppState) -> Result<Value, RpcError> {
    respond(
        list_labs::list_labs(ctx, None, state).await,
        RPC_MSG_LABS_LIST_FAILED,
    )
}

pub(super) async fn inspect(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let request: data::InspectRequest = parse(params, RPC_MSG_INVALID_PARAMS_LAB_ID)?;
    respond(
        inspect::inspect_lab(ctx, &request.lab_id, state).await,
        RPC_MSG_LAB_INSPECT_FAILED,
    )
}

pub(super) async fn download(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let lab = lab_params(params)?;
    respond(
        download::download_lab_files(ctx, &lab.lab_id, state).await,
        RPC_MSG_DOWNLOAD_FAILED,
    )
}

pub(super) async fn down(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let lab = lab_params(params)?;
    respond(
        down::shutdown_lab_nodes(ctx, &lab.lab_id, lab.node_name.as_deref(), state).await,
        RPC_MSG_LAB_DOWN_FAILED,
    )
}

pub(super) async fn cancel(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let lab = lab_params(params)?;
    respond(
        cancel::cancel_lab_operations(ctx, &lab.lab_id, state).await,
        RPC_MSG_LAB_CANCEL_FAILED,
    )
}

pub(super) async fn node_secrets(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let request: data::NodeSecretsRequest =
        parse_quietly(params, RPC_MSG_INVALID_PARAMS_NODE_SECRETS)?;
    respond(
        node_secrets::update_node_secrets(ctx, request, state).await,
        RPC_MSG_NODE_SECRETS_FAILED,
    )
}

pub(super) async fn node_host_keys(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let request: data::NodeHostKeysRequest = parse(params, RPC_MSG_INVALID_PARAMS_NODE_HOST_KEYS)?;
    respond(
        host_keys::refresh_node_host_key(ctx, &request, state).await,
        RPC_MSG_NODE_HOST_KEYS_FAILED,
    )
}

/// Numbers out of range for their field are refused when the request is parsed, never
/// wrapped.
pub(super) async fn update_impairment(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let request: data::UpdateImpairmentRequest = parse(params, RPC_MSG_INVALID_PARAMS_IMPAIRMENT)?;
    respond(
        impairment::update_impairment(ctx, request, state).await,
        RPC_MSG_IMPAIRMENT_UPDATE_FAILED,
    )
}

pub(super) async fn clean(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    let lab = lab_params(params)?;
    respond(
        clean::clean_lab(ctx, &lab.lab_id, state).await,
        RPC_MSG_LAB_CLEAN_FAILED,
    )
}

/// `up` and `redeploy` parse quietly: their requests carry secret file contents.
pub(super) async fn up(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
    progress: ProgressSender,
) -> Result<Value, RpcError> {
    let request: data::UpRequest = parse_quietly(params, RPC_MSG_INVALID_PARAMS_MANIFEST)?;
    // `up` reserves the lab ID by inserting its record, so a second `up` of the same lab fails
    // there, before it creates anything.
    respond(
        up::up_lab(ctx, request, state, progress).await,
        RPC_MSG_LAB_UP_FAILED,
    )
}

pub(super) async fn destroy(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
    progress: ProgressSender,
) -> Result<Value, RpcError> {
    let request: data::DestroyRequest = parse(params, RPC_MSG_INVALID_PARAMS_LAB_ID)?;
    respond(
        destroy::destroy_lab(ctx, &request.lab_id, state, progress).await,
        RPC_MSG_LAB_DESTROY_FAILED,
    )
}

pub(super) async fn resume(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
    progress: ProgressSender,
) -> Result<Value, RpcError> {
    let tailscale: TailscaleParams = parse(params.clone(), RPC_MSG_INVALID_TAILSCALE_KEY)?;
    let lab = lab_params(params)?;
    respond(
        resume::start_lab_nodes(
            ctx,
            &lab.lab_id,
            lab.node_name.as_deref(),
            tailscale.tailscale_auth_key.as_ref(),
            state,
            Some(&progress),
        )
        .await,
        RPC_MSG_LAB_RESUME_FAILED,
    )
}

pub(super) async fn redeploy(
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
    progress: ProgressSender,
) -> Result<Value, RpcError> {
    let request: data::RedeployRequest = parse_quietly(params, RPC_MSG_INVALID_PARAMS_REDEPLOY)?;
    respond(
        redeploy::redeploy_node(ctx, request, state, progress).await,
        RPC_MSG_REDEPLOY_FAILED,
    )
}
