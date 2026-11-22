//! Stopping a lab's operations on the user's request (`cancel`).
//!
//! A lab's lease is held by `up`, `redeploy`, `resume`, `down` and the startup recovery of a
//! lab, under the shared lease; and by a destroy, a clean, a requested stop and the scanner's
//! observation, under the write lease (see `lab_operations`). A requested stop ends only an
//! `up`, `redeploy` or `resume`; when nothing but a `down`, a recovery, a destroy, a clean or
//! the scanner holds the lab, it answers `stopped: false` at once and stops nothing.

use anyhow::Result;
use tracing::instrument;

use shared::data::CancelLabResponse;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::lab_operations;

/// Stop the `up`, `redeploy` and `resume` running on a lab at their next safe point, and wait
/// until they have ended. A stopped `up` rolls back what it created, as a failed one does.
/// `stopped` is true only when one of them held or waited for the lab.
///
/// The caller must own the lab or be an admin.
#[instrument(skip(ctx, state), fields(username = %ctx.username, %lab_id))]
pub async fn cancel_lab_operations(
    ctx: &AuthContext,
    lab_id: &str,
    state: &AppState,
) -> Result<CancelLabResponse> {
    require_lab_access(ctx, &state.db, lab_id).await?;
    let stopped = lab_operations::request_stop(lab_id).await;
    tracing::info!(lab_id = %lab_id, stopped, "Lab operations stop requested");
    Ok(CancelLabResponse {
        lab_id: lab_id.to_owned(),
        stopped,
    })
}
