//! REST image operations.

use super::images_web::parse_upload_fields;
use super::labs_api::require_admin_auth;
use axum::Json;
use axum::extract::{Multipart, Path, State};
use axum::response::{IntoResponse, sse};
use std::collections::HashMap;
use std::env;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::api::request_parse::{JsonBody, QueryParams};
use crate::api::sse::json_progress_stream;
use crate::daemon::state::AppState;
use crate::services::progress::ProgressSender;
use crate::services::{container_pull, delete, import};

use crate::api::errors::ApiError;
use crate::api::extractors::AuthenticatedUser;

use shared::data::{
    ContainerPullRequest, DeleteImageRequest, DownloadImageRequest, ImportRequest,
    ListImagesRequest, ModelKind, NodeKind, NodeModel, ScanImagesRequest, SetDefaultImageRequest,
    ShowImageRequest,
};

/// The model and kind an image path names, refused as a bad request when the model is unknown
/// (an old model name gets the message naming its replacement), the kind is unknown, or the
/// model does not take the kind.
pub(super) fn path_model_kind(model: &str, kind: &str) -> Result<ModelKind, ApiError> {
    let model: NodeModel = model.parse().map_err(ApiError::bad_request)?;
    let kind: NodeKind = kind.parse().map_err(ApiError::bad_request)?;
    ModelKind::new(model, Some(kind)).map_err(|e| ApiError::bad_request(e.to_string()))
}

/// List images
///
/// GET `/api/v1/images`
pub async fn list_images_json(
    _auth: AuthenticatedUser,
    State(state): State<AppState>,
    QueryParams(params): QueryParams<ListImagesRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let response = import::list_images(params, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(e, "Failed to serialize response")
    })?))
}

/// Show image details
///
/// GET `/api/v1/images/{model}/{kind}`, with an optional `?version=`; without one, the
/// default version of the model and kind.
pub async fn show_image_json(
    _auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path((model, kind)): Path<(String, String)>,
    QueryParams(params): QueryParams<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let model_kind = path_model_kind(&model, &kind)?;

    let request = ShowImageRequest {
        model: model_kind.model(),
        kind: Some(model_kind.kind()),
        version: params.get("version").cloned(),
    };

    let response = import::show_image(request, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(e, "Failed to serialize response")
    })?))
}

/// Import an image (SSE streaming)
///
/// POST `/api/v1/images/import`
pub async fn import_image_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    JsonBody(request): JsonBody<ImportRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin_auth(&auth)?;

    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let (result_tx, result_rx) = oneshot::channel();
    let progress = ProgressSender::new(progress_tx);

    tokio::spawn(async move {
        let result = import::import_image(request, &state, progress).await;
        let _ = result_tx.send(result);
    });

    Ok(sse::Sse::new(json_progress_stream(progress_rx, result_rx)))
}

/// Upload an image via multipart form data: `model`, `version`, `file`, and the optional
/// `kind` (absent or empty, the model's default kind) and `default` (`on`).
///
/// POST `/api/v1/images/upload`
pub async fn upload_image_multipart(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin_auth(&auth)?;

    let fields = parse_upload_fields(multipart)
        .await
        .map_err(ApiError::bad_request)?;

    // The name holds nothing the caller sent: the version is validated only by the import.
    let temp_file = env::temp_dir().join(format!("sherpa_upload_{}", Uuid::new_v4()));
    let temp_path = temp_file.to_string_lossy().to_string();

    tokio::fs::write(&temp_file, &fields.file_data)
        .await
        .map_err(|e| ApiError::internal(e, "Failed to write temp file"))?;

    let request = ImportRequest {
        model: fields.model_kind.model(),
        kind: Some(fields.model_kind.kind()),
        version: fields.version,
        src: temp_path.clone(),
        default: fields.default,
    };

    let (progress_tx, _progress_rx) = mpsc::unbounded_channel();
    let progress = ProgressSender::new(progress_tx);

    let result = import::import_image(request, &state, progress).await;

    // Clean up temp file regardless of result
    if let Err(e) = tokio::fs::remove_file(&temp_file).await {
        tracing::warn!("Failed to clean up temp file {}: {:?}", temp_path, e);
    }

    let response = result?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(e, "Failed to serialize response")
    })?))
}

/// Delete an image
///
/// DELETE `/api/v1/images/{model}/{kind}/{version}`
pub async fn delete_image_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path((model, kind, version)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin_auth(&auth)?;

    let model_kind = path_model_kind(&model, &kind)?;

    let request = DeleteImageRequest {
        model: model_kind.model(),
        kind: Some(model_kind.kind()),
        version,
    };

    let response = delete::delete_image(request, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(e, "Failed to serialize response")
    })?))
}

/// Set default image version
///
/// POST `/api/v1/images/{model}/{kind}/{version}/default`
pub async fn set_default_image_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path((model, kind, version)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin_auth(&auth)?;

    let model_kind = path_model_kind(&model, &kind)?;

    let request = SetDefaultImageRequest {
        model: model_kind.model(),
        kind: Some(model_kind.kind()),
        version,
    };

    let response = import::set_default_image(request, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(e, "Failed to serialize response")
    })?))
}

/// Scan for images
///
/// POST `/api/v1/admin/tools/images/scan`
pub async fn scan_images_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    JsonBody(request): JsonBody<ScanImagesRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin_auth(&auth)?;

    let response = import::scan_images(request, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(e, "Failed to serialize response")
    })?))
}

/// Pull a container image (SSE streaming)
///
/// POST `/api/v1/images/pull`
pub async fn pull_image_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    JsonBody(request): JsonBody<ContainerPullRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin_auth(&auth)?;

    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let (result_tx, result_rx) = oneshot::channel();
    let progress = ProgressSender::new(progress_tx);

    tokio::spawn(async move {
        let result = container_pull::pull_container_image(request, &state, progress).await;
        let _ = result_tx.send(result);
    });

    Ok(sse::Sse::new(json_progress_stream(progress_rx, result_rx)))
}

/// Download a VM image (SSE streaming)
///
/// POST `/api/v1/images/download`
pub async fn download_image_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    JsonBody(request): JsonBody<DownloadImageRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin_auth(&auth)?;

    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let (result_tx, result_rx) = oneshot::channel();
    let progress = ProgressSender::new(progress_tx);

    tokio::spawn(async move {
        let result = import::download_image(request, &state, progress).await;
        let _ = result_tx.send(result);
    });

    Ok(sse::Sse::new(json_progress_stream(progress_rx, result_rx)))
}

// =============================================================================
// REST API — User Management
// =============================================================================
