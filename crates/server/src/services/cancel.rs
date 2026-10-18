use anyhow::Result;
use tracing::instrument;

use shared::data::CancelLabResponse;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::lab_operations::{self, StopReason};

/// Stop the operations running on a lab (`up`, `redeploy`, `resume`) at their next safe point
/// and wait until they have ended. A stopped `up` rolls back what it created, as a failed one
/// does.
///
/// The caller must own the lab or be an admin.
#[instrument(skip(ctx, state), fields(username = %ctx.username, %lab_id))]
pub async fn cancel_lab_operations(
    ctx: &AuthContext,
    lab_id: &str,
    state: &AppState,
) -> Result<CancelLabResponse> {
    require_lab_access(ctx, &state.db, lab_id).await?;
    let (stopped, _released) = lab_operations::stop(lab_id, StopReason::Request).await;
    tracing::info!(lab_id = %lab_id, stopped, "Stopped lab operations on request");
    Ok(CancelLabResponse {
        lab_id: lab_id.to_owned(),
        stopped,
    })
}
