//! REST image operations.

use super::images_web::parse_upload_fields;
use super::labs_api::require_admin_auth;
use axum::Json;
use axum::extract::{Multipart, Path, Query, State};
use axum::response::{IntoResponse, sse};
use std::collections::HashMap;

use crate::api::sse::json_progress_stream;
use crate::daemon::state::AppState;
use crate::services::progress::ProgressSender;
use crate::services::{container_pull, delete, import};

use crate::api::errors::ApiError;
use crate::api::extractors::AuthenticatedUser;

use shared::data::{
    ContainerPullRequest, DeleteImageRequest, DownloadImageRequest, ImportRequest,
    ListImagesRequest, NodeModel, ScanImagesRequest, SetDefaultImageRequest, ShowImageRequest,
};

/// List images
///
/// GET /api/v1/images
pub async fn list_images_json(
    _auth: AuthenticatedUser,
    State(state): State<AppState>,
    Query(params): Query<ListImagesRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let response = import::list_images(params, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(format!("Failed to serialize response: {e}"))
    })?))
}

/// Show image details
///
/// GET /api/v1/images/{model}
pub async fn show_image_json(
    _auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(model): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let model: NodeModel = model
        .parse()
        .map_err(|_| ApiError::bad_request(format!("Invalid model: {model}")))?;

    let request = ShowImageRequest {
        model,
        version: params.get("version").cloned(),
    };

    let response = import::show_image(request, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(format!("Failed to serialize response: {e}"))
    })?))
}

/// Import an image (SSE streaming)
///
/// POST /api/v1/images/import
pub async fn import_image_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Json(request): Json<ImportRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin_auth(&auth)?;

    let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let progress = ProgressSender::new(progress_tx);

    tokio::spawn(async move {
        let result = import::import_image(request, &state, progress).await;
        let _ = result_tx.send(result);
    });

    Ok(sse::Sse::new(json_progress_stream(progress_rx, result_rx)))
}

/// Upload an image via multipart form data
///
/// POST /api/v1/images/upload
pub async fn upload_image_multipart(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin_auth(&auth)?;

    let fields = parse_upload_fields(multipart)
        .await
        .map_err(ApiError::bad_request)?;

    // Write file data to a temp file
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!(
        "sherpa_upload_{}_{}_{}",
        fields.model,
        fields.version,
        uuid::Uuid::new_v4()
    ));
    let temp_path = temp_file.to_string_lossy().to_string();

    tokio::fs::write(&temp_file, &fields.file_data)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to write temp file: {}", e)))?;

    let request = ImportRequest {
        model: fields.model,
        version: fields.version,
        src: temp_path.clone(),
        default: fields.default,
    };

    let (progress_tx, _progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let progress = ProgressSender::new(progress_tx);

    let result = import::import_image(request, &state, progress).await;

    // Clean up temp file regardless of result
    if let Err(e) = tokio::fs::remove_file(&temp_file).await {
        tracing::warn!("Failed to clean up temp file {}: {:?}", temp_path, e);
    }

    let response = result.map_err(|e| ApiError::internal(format!("Import failed: {}", e)))?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(format!("Failed to serialize response: {e}"))
    })?))
}

/// Delete an image
///
/// DELETE /api/v1/images/{model}/{version}
pub async fn delete_image_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path((model, version)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin_auth(&auth)?;

    let model: NodeModel = model
        .parse()
        .map_err(|_| ApiError::bad_request(format!("Invalid model: {model}")))?;

    let request = DeleteImageRequest { model, version };

    let response = delete::delete_image(request, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(format!("Failed to serialize response: {e}"))
    })?))
}

/// Set default image version
///
/// POST /api/v1/images/{model}/{version}/default
pub async fn set_default_image_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path((model, version)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin_auth(&auth)?;

    let model: NodeModel = model
        .parse()
        .map_err(|_| ApiError::bad_request(format!("Invalid model: {model}")))?;

    let request = SetDefaultImageRequest { model, version };

    let response = import::set_default_image(request, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(format!("Failed to serialize response: {e}"))
    })?))
}

/// Scan for images
///
/// POST /api/v1/admin/tools/images/scan
pub async fn scan_images_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Json(request): Json<ScanImagesRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin_auth(&auth)?;

    let response = import::scan_images(request, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(serde_json::to_value(&response).map_err(|e| {
        ApiError::internal(format!("Failed to serialize response: {e}"))
    })?))
}

/// Pull a container image (SSE streaming)
///
/// POST /api/v1/images/pull
pub async fn pull_image_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Json(request): Json<ContainerPullRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin_auth(&auth)?;

    let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let progress = ProgressSender::new(progress_tx);

    tokio::spawn(async move {
        let result = container_pull::pull_container_image(request, &state, progress).await;
        let _ = result_tx.send(result);
    });

    Ok(sse::Sse::new(json_progress_stream(progress_rx, result_rx)))
}

/// Download a VM image (SSE streaming)
///
/// POST /api/v1/images/download
pub async fn download_image_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Json(request): Json<DownloadImageRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin_auth(&auth)?;

    let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
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
