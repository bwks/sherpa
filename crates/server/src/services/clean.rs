use anyhow::Result;
use opentelemetry::KeyValue;

use shared::data::{DestroyResponse, DestroySummary, LabInfo};
use shared::konst::{LAB_FILE_NAME, SHERPA_LABS_PATH};
use shared::util::load_file;

use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_admin;
use crate::daemon::state::AppState;
use crate::services::destroy::{cleanup_database, cleanup_lab_metadata, destroy_lab_resources};
use crate::services::error::ServiceError;
use crate::services::lab_operations::{self, StopReason};

/// Clean all resources for a lab (admin only)
///
/// Unlike `destroy_lab`, this function:
/// - Does not require the lab to exist in the database
/// - Requires admin privileges instead of lab ownership
/// - Tolerates missing lab info files
/// - Continues infrastructure teardown after partial failures
/// - Retains lab ownership and saved configuration if Tailscale cleanup fails
#[instrument(skip(ctx, state), fields(username = %ctx.username, %lab_id))]
pub async fn clean_lab(
    ctx: &AuthContext,
    lab_id: &str,
    state: &AppState,
) -> Result<DestroyResponse> {
    require_admin(ctx)?;
    // Stop operations still building or changing the lab before removing its resources.
    let (_, _stopped) = lab_operations::stop(lab_id, StopReason::Destroy).await;
    clean_lab_resources(lab_id, state).await
}

/// Clean all resources for a lab without any authorization check.
///
/// Only for server-internal callers, such as rolling back a failed `up` on behalf of the
/// user who started it. Requests from users go through `clean_lab`.
#[instrument(skip(state), fields(%lab_id))]
pub(crate) async fn clean_lab_resources(lab_id: &str, state: &AppState) -> Result<DestroyResponse> {
    // Resources are matched by lab ID and the lab folder is built from it, so an empty or
    // path-like ID would reach other labs' resources.
    db::validate_lab_id(lab_id).map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    let start_time = std::time::Instant::now();

    tracing::info!(lab_id = %lab_id, "Starting admin clean operation");

    let mut summary = DestroySummary::default();
    let mut errors = Vec::new();

    // Try to load lab name from filesystem, fall back to "unknown"
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let lab_name = load_file(&format!("{lab_dir}/{LAB_FILE_NAME}"))
        .ok()
        .and_then(|content| content.parse::<LabInfo>().ok().map(|info| info.name))
        .unwrap_or_else(|| "unknown".to_string());

    tracing::debug!(
        lab_id = %lab_id,
        lab_name = %lab_name,
        lab_dir = %lab_dir,
        "Clean operation context"
    );

    destroy_lab_resources(lab_id, state, &mut summary, &mut errors, None).await;

    cleanup_lab_metadata(
        lab_id,
        &lab_dir,
        cleanup_database(lab_id, &state.db),
        &mut summary,
        &mut errors,
        None,
    )
    .await;

    let success = errors.is_empty();
    let total_duration = start_time.elapsed().as_secs();

    let op_attrs = &[KeyValue::new("operation.type", "clean")];
    state
        .metrics
        .operation_duration
        .record(start_time.elapsed().as_secs_f64(), op_attrs);
    if !success {
        state.metrics.error_count.add(1, op_attrs);
    }

    tracing::info!(
        lab_id = %lab_id,
        lab_name = %lab_name,
        success = success,
        total_duration_secs = total_duration,
        total_errors = errors.len(),
        "Admin clean operation completed"
    );

    Ok(DestroyResponse {
        success,
        lab_id: lab_id.to_string(),
        lab_name,
        summary,
        errors,
    })
}
