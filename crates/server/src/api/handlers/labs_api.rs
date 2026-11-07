//! REST lab operations.

use anyhow::Context;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, sse};
use serde::Deserialize;
use std::future::Future;

use crate::api::sse::json_progress_stream;
use crate::auth::policy::{require_lab_access, require_lab_owner, require_lab_starter};
use crate::daemon::state::AppState;
use crate::services::progress::ProgressSender;
use crate::services::{
    cancel, clean, destroy, down, impairment, inspect, list_labs, redeploy, resume, up,
};

use crate::api::errors::ApiError;
use crate::api::extractors::AuthenticatedUser;

use shared::data::{
    CancelLabResponse, InspectResponse, LabNodeActionResponse, ListLabsResponse, RedeployRequest,
    TailnetAuthKey, UpRequest, UpdateImpairmentRequest, UpdateImpairmentResponse,
};

/// Get detailed information about a lab
///
/// Returns the current state of a lab including:
/// - Lab metadata (name, topology, description)
/// - Active devices with status, management IPs, and disk information
/// - Inactive devices (nodes that should exist but aren't running)
///
/// # Authentication
/// Requires valid JWT token in `Authorization: Bearer <token>` header.
///
/// # Authorization
/// Users can only inspect their own labs. Admins can inspect any lab.
///
/// # Path Parameters
/// - `id` - The lab ID to inspect
///
/// # Response (200 OK)
/// Returns `InspectResponse` with lab details.
///
/// # Errors
/// - `401 Unauthorized` - Missing or invalid token
/// - `403 Forbidden` - User doesn't own this lab
/// - `404 Not Found` - Lab doesn't exist
/// - `500 Internal Server Error` - Server-side error
///
pub async fn get_lab(
    State(state): State<AppState>,
    Path(lab_id): Path<String>,
    auth: AuthenticatedUser,
) -> Result<Json<InspectResponse>, ApiError> {
    let auth = auth.into_context();
    tracing::debug!(
        "User '{}' requesting inspect for lab '{}'",
        auth.username,
        lab_id
    );

    // Call service layer
    let response = inspect::inspect_lab(&auth, &lab_id, &state)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to inspect lab '{}' for user '{}': {:?}",
                lab_id,
                auth.username,
                e
            );
            ApiError::from(e)
        })?;

    tracing::info!(
        "Successfully inspected lab '{}' for user '{}'",
        lab_id,
        auth.username
    );

    Ok(Json(response))
}

/// List all labs for a specific user (JSON API)
///
/// Query all labs owned by the specified user.
/// Returns JSON response for programmatic API access.
/// Publicly accessible (no authentication) for initial development.
///
/// # Query Parameters
/// - `username` (required) - The username to list labs for
///
/// # Response (200 OK)
/// Returns `ListLabsResponse` with lab summaries
///
/// # Errors
/// - `404 Not Found` - User doesn't exist
/// - `500 Internal Server Error` - Database error
///
pub async fn get_labs_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Query(params): Query<ListLabsQuery>,
) -> Result<Json<ListLabsResponse>, ApiError> {
    let auth = auth.into_context();
    let username = params.username.as_deref().unwrap_or(&auth.username).trim();

    tracing::debug!("Listing labs for user '{}'", username);

    // Call service layer; it allows admins to list other users' labs
    let response = list_labs::list_labs(&auth, Some(username), &state)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list labs for user '{}': {:?}", username, e);
            ApiError::from(e)
        })?;

    tracing::info!(
        "Successfully listed {} labs for user '{}'",
        response.total,
        username
    );

    Ok(Json(response))
}

/// Run a lab operation as its own task and wait for its result.
///
/// axum drops a handler's future when its client disconnects. Running the operation in a
/// separate task means a disconnect only stops the wait: the operation carries on to the end
/// instead of stopping at an arbitrary point mid-step.
pub(super) async fn run_detached<T, F>(operation: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: Future<Output = anyhow::Result<T>> + Send + 'static,
{
    tokio::spawn(operation)
        .await
        .context("Lab operation task failed")?
}

/// Optional body for down/resume operations
#[derive(Deserialize, Default)]
pub struct NodeActionPayload {
    pub node_name: Option<String>,
    /// Resume only: re-enrolls the lab's Tailscale gateway. Requires server TLS.
    pub tailscale_auth_key: Option<TailnetAuthKey>,
}

/// Helper to require admin privileges
pub(super) fn require_admin_auth(auth: &AuthenticatedUser) -> Result<(), ApiError> {
    if !auth.is_admin {
        return Err(ApiError::forbidden("Admin privileges required"));
    }
    Ok(())
}

/// Create a lab (SSE streaming)
///
/// POST /api/v1/labs
pub async fn create_lab_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Json(payload): Json<UpRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let auth = auth.into_context();
    // Checked before the stream opens so a refusal is a 403, not an error event.
    require_lab_starter(&auth)?;

    let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let progress = ProgressSender::new(progress_tx);

    tokio::spawn(async move {
        let result = up::up_lab(&auth, payload, &state, progress).await;
        let _ = result_tx.send(result);
    });

    Ok(sse::Sse::new(json_progress_stream(progress_rx, result_rx)))
}

/// Destroy a lab (SSE streaming)
///
/// DELETE /api/v1/labs/{lab_id}
pub async fn delete_lab_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(lab_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let auth = auth.into_context();
    // Checked before the stream opens so a refusal is a 403, not an error event.
    require_lab_access(&auth, &state.db, &lab_id).await?;

    let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let progress = ProgressSender::new(progress_tx);

    tokio::spawn(async move {
        let result = destroy::destroy_lab(&auth, &lab_id, &state, progress).await;
        let _ = result_tx.send(result);
    });

    Ok(sse::Sse::new(json_progress_stream(progress_rx, result_rx)))
}

/// Stop lab nodes
///
/// POST /api/v1/labs/{lab_id}/down
pub async fn down_lab_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(lab_id): Path<String>,
    Json(payload): Json<NodeActionPayload>,
) -> Result<Json<LabNodeActionResponse>, ApiError> {
    let auth = auth.into_context();
    let response = run_detached(async move {
        down::shutdown_lab_nodes(&auth, &lab_id, payload.node_name.as_deref(), &state).await
    })
    .await
    .map_err(ApiError::from)?;

    Ok(Json(response))
}

/// Stop the operations running on a lab and wait for them to end
///
/// POST /api/v1/labs/{lab_id}/cancel
pub async fn cancel_lab_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(lab_id): Path<String>,
) -> Result<Json<CancelLabResponse>, ApiError> {
    let auth = auth.into_context();
    let response =
        run_detached(async move { cancel::cancel_lab_operations(&auth, &lab_id, &state).await })
            .await
            .map_err(ApiError::from)?;

    Ok(Json(response))
}

/// Resume lab nodes
///
/// POST /api/v1/labs/{lab_id}/resume
pub async fn resume_lab_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(lab_id): Path<String>,
    Json(payload): Json<NodeActionPayload>,
) -> Result<Json<LabNodeActionResponse>, ApiError> {
    let auth = auth.into_context();
    let response = run_detached(async move {
        resume::start_lab_nodes(
            &auth,
            &lab_id,
            payload.node_name.as_deref(),
            payload.tailscale_auth_key.as_ref(),
            &state,
            None,
        )
        .await
    })
    .await
    .map_err(ApiError::from)?;

    Ok(Json(response))
}

/// Redeploy a lab node (SSE streaming)
///
/// POST /api/v1/labs/{lab_id}/nodes/{node_name}/redeploy
pub async fn redeploy_node_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path((lab_id, node_name)): Path<(String, String)>,
    Json(payload): Json<serde_json::Value>,
) -> Result<impl IntoResponse, ApiError> {
    let auth = auth.into_context();
    // Checked before the body is validated and the stream opens, so a non-owner gets 403.
    require_lab_owner(&auth, &state.db, &lab_id).await?;

    let manifest = payload
        .get("manifest")
        .cloned()
        .ok_or_else(|| ApiError::bad_request("Missing 'manifest' field in request body"))?;
    let secrets = match payload.get("secrets") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            serde_json::from_value(value.clone())
                .map_err(|_| ApiError::bad_request("Invalid 'secrets' field in request body"))?,
        ),
    };

    let request = RedeployRequest {
        lab_id,
        node_name,
        manifest,
        secrets,
    };

    let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let progress = ProgressSender::new(progress_tx);

    tokio::spawn(async move {
        let result = redeploy::redeploy_node(&auth, request, &state, progress).await;
        let _ = result_tx.send(result);
    });

    Ok(sse::Sse::new(json_progress_stream(progress_rx, result_rx)))
}

/// Force-clean a lab (admin only)
///
/// POST /api/v1/admin/tools/labs/clean/{lab_id}
pub async fn clean_lab_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path(lab_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let auth = auth.into_context();
    let response = run_detached(async move { clean::clean_lab(&auth, &lab_id, &state).await })
        .await
        .map_err(ApiError::from)?;

    Ok(Json(response))
}

// =============================================================================
// REST API — Image Management
// =============================================================================

/// Update link impairment on a running P2p link
///
/// POST /api/v1/labs/{lab_id}/links/{link_index}/impairment
pub async fn update_impairment_json(
    auth: AuthenticatedUser,
    State(state): State<AppState>,
    Path((lab_id, link_index)): Path<(String, u16)>,
    Json(payload): Json<UpdateImpairmentRequest>,
) -> Result<Json<UpdateImpairmentResponse>, ApiError> {
    let request = UpdateImpairmentRequest {
        lab_id,
        link_index,
        ..payload
    };

    let response = impairment::update_impairment(&auth.into_context(), request, &state)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(response))
}

/// Query parameters for listing labs
#[derive(Deserialize)]
pub struct ListLabsQuery {
    /// Whose labs to list. Defaults to the caller; other users require admin privileges.
    pub username: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_dropped_handler_does_not_stop_its_operation() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let waiting = tokio::spawn(run_detached(async move {
            let _ = started_tx.send(());
            let _ = release_rx.await;
            let _ = finished_tx.send(());
            Ok(())
        }));
        started_rx.await.unwrap();
        // The client disconnects: axum drops the handler that is waiting.
        waiting.abort();
        assert!(waiting.await.unwrap_err().is_cancelled());
        release_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), finished_rx)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn run_detached_returns_the_operation_result() {
        assert_eq!(run_detached(async { Ok(7) }).await.unwrap(), 7);
        let error = run_detached(async { Err::<(), _>(anyhow::anyhow!("boom")) })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("boom"));
    }
}
