use anyhow::{Context, Result};
use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::policy::{AccessError, require_self_or_admin};
use crate::daemon::state::AppState;
use shared::data::{LabSummary, ListLabsResponse};

/// List the labs owned by a user
///
/// # Arguments
/// * `ctx` - The authenticated caller
/// * `username` - Whose labs to list; `None` lists the caller's own labs. Listing another
///   user's labs requires admin privileges.
/// * `state` - Application state with database connection
///
/// # Returns
/// A `ListLabsResponse` containing lab summaries
///
/// # Errors
/// Returns error if:
/// - The caller may not list that user's labs
/// - User doesn't exist in database
/// - Database query fails
#[instrument(skip(ctx, state), fields(caller = %ctx.username, ?username))]
pub async fn list_labs(
    ctx: &AuthContext,
    username: Option<&str>,
    state: &AppState,
) -> Result<ListLabsResponse> {
    let username = username.unwrap_or(&ctx.username);
    require_self_or_admin(ctx, username)?;

    let user = db::find_user(&state.db, username)
        .await
        .context("Failed to look up user")?
        .ok_or_else(|| AccessError::NotFound(format!("User not found: {username}")))?;

    // Get user's record ID
    let user_id = user.id.ok_or_else(|| anyhow::anyhow!("User has no ID"))?;

    // Get all labs for this user
    let labs = db::list_labs_by_user(&state.db, user_id.clone())
        .await
        .context("Failed to list labs for user")?;

    // Build lab summaries — status is read directly from the lab record
    let mut lab_summaries = Vec::new();

    for lab in labs {
        let lab_record_id = lab
            .id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Lab has no ID"))?;

        let node_count = db::count_nodes_by_lab(&state.db, lab_record_id)
            .await
            .unwrap_or(0);

        lab_summaries.push(LabSummary {
            id: lab.lab_id.clone(),
            name: lab.name.clone(),
            node_count,
            status: lab.status,
        });
    }

    let total = lab_summaries.len();

    Ok(ListLabsResponse {
        labs: lab_summaries,
        total,
    })
}
