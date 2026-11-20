//! Image RPC methods. Access is checked by the dispatcher: listing and showing need a user,
//! the rest an admin.

use serde_json::Value;

use shared::data;

use super::dispatch::{parse, respond};
use crate::api::websocket::messages::RpcError;
use crate::daemon::state::AppState;
use crate::services::progress::ProgressSender;
use crate::services::{container_pull, delete, import};

const RPC_MSG_IMAGE_IMPORT_FAILED: &str = "Image import operation failed";
const RPC_MSG_IMAGE_LIST_FAILED: &str = "Image list operation failed";
const RPC_MSG_IMAGE_SHOW_FAILED: &str = "Image show operation failed";
const RPC_MSG_INVALID_PARAMS_IMAGE_SHOW: &str = "Invalid params: expected ShowImageRequest";
const RPC_MSG_IMAGE_SCAN_FAILED: &str = "Image scan operation failed";
const RPC_MSG_IMAGE_DELETE_FAILED: &str = "Image delete operation failed";
const RPC_MSG_INVALID_PARAMS_IMAGE_DELETE: &str = "Invalid params: expected DeleteImageRequest";
const RPC_MSG_IMAGE_SET_DEFAULT_FAILED: &str = "Image set-default operation failed";
const RPC_MSG_INVALID_PARAMS_IMAGE_SET_DEFAULT: &str =
    "Invalid params: expected SetDefaultImageRequest";
const RPC_MSG_CONTAINER_PULL_FAILED: &str = "Container image pull operation failed";
const RPC_MSG_INVALID_PARAMS_CONTAINER_PULL: &str = "Invalid params: expected ContainerPullRequest";
const RPC_MSG_IMAGE_DOWNLOAD_FAILED: &str = "Image download operation failed";
const RPC_MSG_INVALID_PARAMS_IMAGE_DOWNLOAD: &str = "Invalid params: expected DownloadImageRequest";
const RPC_MSG_INVALID_PARAMS_IMPORT: &str = "Invalid params: expected ImportRequest";
const RPC_MSG_INVALID_PARAMS_IMAGE_LIST: &str = "Invalid params: expected ListImagesRequest";

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
    let request = import_request(params)?;
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

/// The import request in `params`, without the `token` the dispatcher authenticated the call
/// with: `ImportRequest` denies unknown fields, so a removed image setting is refused naming it.
fn import_request(mut params: Value) -> Result<data::ImportRequest, RpcError> {
    if let Some(fields) = params.as_object_mut() {
        fields.remove("token");
    }
    parse(params, RPC_MSG_INVALID_PARAMS_IMPORT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn image_import_015_an_import_with_a_field_the_server_does_not_take_is_refused_naming_it() {
        let request = import_request(json!({
            "model": "generic",
            "kind": "unikernel",
            "version": "1",
            "src": "/var/tmp/kernel.elf",
            "token": "jwt",
        }))
        .unwrap();
        assert_eq!(request.kind, Some(data::NodeKind::Unikernel));

        let error = import_request(json!({
            "model": "ubuntu_linux",
            "version": "1",
            "src": "/var/tmp/a.qcow2",
            "token": "jwt",
            "ztp_password": "x",
        }))
        .unwrap_err();
        assert!(
            error
                .context
                .is_some_and(|context| context.contains("unknown field `ztp_password`")),
        );
    }
}
