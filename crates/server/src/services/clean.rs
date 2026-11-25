use std::time::Instant;

use anyhow::Result;
use opentelemetry::KeyValue;

use shared::data::{DestroyResponse, DestroySummary};
use shared::konst::SHERPA_LABS_PATH;

use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_admin;
use crate::daemon::state::AppState;
use crate::services::destroy::tear_down_lab;
use crate::services::error::ServiceError;
use crate::services::lab_operations::{self, LabHold};

/// The name reported for a lab with no record.
const UNKNOWN_LAB_NAME: &str = "unknown";

/// Clean all resources for a lab (admin only)
///
/// Unlike `destroy_lab`, this function:
/// - Does not require the lab to exist in the database: resources and a folder left with no
///   record are removed, and that is a success
/// - Requires admin privileges instead of lab ownership
///
/// Like it, it continues infrastructure teardown after partial failures, and keeps the lab's
/// record, `DestroyFailed`, when anything is left behind.
#[instrument(skip(ctx, state), fields(username = %ctx.username, %lab_id))]
pub async fn clean_lab(
    ctx: &AuthContext,
    lab_id: &str,
    state: &AppState,
) -> Result<DestroyResponse> {
    require_admin(ctx)?;
    // Stop operations still building or changing the lab before removing its resources.
    let (_, stopped) = lab_operations::stop(lab_id).await;
    clean_lab_resources(state, &stopped).await
}

/// Clean all resources of the lab `hold` is on, without any authorization check.
///
/// Only for server-internal callers, such as rolling back a failed `up` on behalf of the
/// user who started it. Requests from users go through `clean_lab`.
#[instrument(skip(state, hold), fields(lab_id = %hold.lab_id()))]
pub(crate) async fn clean_lab_resources(
    state: &AppState,
    hold: &impl LabHold,
) -> Result<DestroyResponse> {
    let lab_id = hold.lab_id();
    // Resources are matched by lab ID and the lab folder is built from it, so an empty or
    // path-like ID would reach other labs' resources.
    db::validate_lab_id(lab_id).map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    let start_time = Instant::now();

    tracing::info!(lab_id = %lab_id, "Starting admin clean operation");

    let mut summary = DestroySummary::default();
    let mut errors = Vec::new();

    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let lab_name = lab_name(&state.db, lab_id).await;

    tracing::debug!(
        lab_id = %lab_id,
        lab_name = %lab_name,
        lab_dir = %lab_dir,
        "Clean operation context"
    );

    tear_down_lab(state, hold, &mut summary, &mut errors, None).await;

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

/// The name of the lab `lab_id` from its record, or `unknown` for a lab with no record, which
/// clean removes too. A failed lookup is logged and also gives `unknown`.
#[instrument(skip(db), level = "debug")]
async fn lab_name(db: &db::Database, lab_id: &str) -> String {
    match db::find_lab(db, lab_id).await {
        Ok(Some(lab)) => lab.name,
        Ok(None) => {
            tracing::debug!("Lab has no record; cleaning what is left of it");
            UNKNOWN_LAB_NAME.to_owned()
        }
        Err(error) => {
            tracing::warn!(
                error = format!("{error:#}"),
                "Failed to look up the lab's name"
            );
            UNKNOWN_LAB_NAME.to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use shared::data::NodeModel;

    use crate::services::lab_definition::tests::lab_with;

    use super::*;

    #[tokio::test]
    async fn clean_names_the_lab_from_its_record() {
        let (db, lab) = lab_with(&[("vm1", NodeModel::AlpineLinux)]).await;
        assert_eq!(lab_name(&db, &lab.lab_id).await, "core");
    }

    #[tokio::test]
    async fn clean_names_a_lab_with_no_record_unknown() {
        let (db, _) = lab_with(&[]).await;
        assert_eq!(lab_name(&db, "ffff0000").await, UNKNOWN_LAB_NAME);
    }
}
