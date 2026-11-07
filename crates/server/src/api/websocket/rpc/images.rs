//! Image RPC methods. Access is checked by the dispatcher: listing and showing need a user,
//! the rest an admin.

use serde_json::Value;

use shared::data;
use shared::konst::{
    RPC_MSG_CONTAINER_PULL_FAILED, RPC_MSG_IMAGE_DELETE_FAILED, RPC_MSG_IMAGE_DOWNLOAD_FAILED,
    RPC_MSG_IMAGE_IMPORT_FAILED, RPC_MSG_IMAGE_LIST_FAILED, RPC_MSG_IMAGE_SCAN_FAILED,
    RPC_MSG_IMAGE_SET_DEFAULT_FAILED, RPC_MSG_IMAGE_SHOW_FAILED,
    RPC_MSG_INVALID_PARAMS_CONTAINER_PULL, RPC_MSG_INVALID_PARAMS_IMAGE_DELETE,
    RPC_MSG_INVALID_PARAMS_IMAGE_DOWNLOAD, RPC_MSG_INVALID_PARAMS_IMAGE_LIST,
    RPC_MSG_INVALID_PARAMS_IMAGE_SET_DEFAULT, RPC_MSG_INVALID_PARAMS_IMAGE_SHOW,
    RPC_MSG_INVALID_PARAMS_IMPORT,
};

use super::dispatch::{parse, respond};
use crate::api::websocket::messages::RpcError;
use crate::daemon::state::AppState;
use crate::services::progress::ProgressSender;
use crate::services::{container_pull, delete, import};

const RPC_MSG_INVALID_PARAMS_IMAGE_SCAN: &str = "Invalid params: expected ScanImagesRequest";

pub(super) async fn list(params: Value, state: &AppState) -> Result<Value, RpcError> {
    let request: data::ListImagesRequest = parse(params, RPC_MSG_INVALID_PARAMS_IMAGE_LIST)?;
    respond(
        import::list_images(request, state).await,
        RPC_MSG_IMAGE_LIST_FAILED,
    )
}

pub(super) async fn show(params: Value, state: &AppState) -> Result<Value, RpcError> {
    let request: data::ShowImageRequest = parse(params, RPC_MSG_INVALID_PARAMS_IMAGE_SHOW)?;
    respond(
        import::show_image(request, state).await,
        RPC_MSG_IMAGE_SHOW_FAILED,
    )
}

pub(super) async fn delete(params: Value, state: &AppState) -> Result<Value, RpcError> {
    let request: data::DeleteImageRequest = parse(params, RPC_MSG_INVALID_PARAMS_IMAGE_DELETE)?;
    respond(
        delete::delete_image(request, state).await,
        RPC_MSG_IMAGE_DELETE_FAILED,
    )
}

pub(super) async fn set_default(params: Value, state: &AppState) -> Result<Value, RpcError> {
    let request: data::SetDefaultImageRequest =
        parse(params, RPC_MSG_INVALID_PARAMS_IMAGE_SET_DEFAULT)?;
    respond(
        import::set_default_image(request, state).await,
        RPC_MSG_IMAGE_SET_DEFAULT_FAILED,
    )
}

pub(super) async fn scan(params: Value, state: &AppState) -> Result<Value, RpcError> {
    let request: data::ScanImagesRequest = parse(params, RPC_MSG_INVALID_PARAMS_IMAGE_SCAN)?;
    respond(
        import::scan_images(request, state).await,
        RPC_MSG_IMAGE_SCAN_FAILED,
    )
}

pub(super) async fn import(
    params: Value,
    state: &AppState,
    progress: ProgressSender,
) -> Result<Value, RpcError> {
    let request: data::ImportRequest = parse(params, RPC_MSG_INVALID_PARAMS_IMPORT)?;
    respond(
        import::import_image(request, state, progress).await,
        RPC_MSG_IMAGE_IMPORT_FAILED,
    )
}

pub(super) async fn pull(
    params: Value,
    state: &AppState,
    progress: ProgressSender,
) -> Result<Value, RpcError> {
    let request: data::ContainerPullRequest = parse(params, RPC_MSG_INVALID_PARAMS_CONTAINER_PULL)?;
    respond(
        container_pull::pull_container_image(request, state, progress).await,
        RPC_MSG_CONTAINER_PULL_FAILED,
    )
}

pub(super) async fn download(
    params: Value,
    state: &AppState,
    progress: ProgressSender,
) -> Result<Value, RpcError> {
    let request: data::DownloadImageRequest = parse(params, RPC_MSG_INVALID_PARAMS_IMAGE_DOWNLOAD)?;
    respond(
        import::download_image(request, state, progress).await,
        RPC_MSG_IMAGE_DOWNLOAD_FAILED,
    )
}
