use anyhow::{Context, Result};

use crate::daemon::state::AppState;
use shared::data::{LabStatus, LabSummary, ListLabsResponse};

/// List all labs for a specific user
///
/// # Arguments
/// * `username` - The username to list labs for
/// * `state` - Application state with database connection
///
/// # Returns
/// A `ListLabsResponse` containing lab summaries
///
/// # Errors
/// Returns error if:
/// - User doesn't exist in database
/// - Database query fails
pub async fn list_labs(username: &str, state: &AppState) -> Result<ListLabsResponse> {
    // Get user from database
    let user = db::get_user(&state.db, username)
        .await
        .context(format!("User not found: {}", username))?;

    // Get user's record ID
    let user_id = user.id.ok_or_else(|| anyhow::anyhow!("User has no ID"))?;

    // Get all labs for this user
    let labs = db::list_labs_by_user(&state.db, user_id.clone())
        .await
        .context("Failed to list labs for user")?;

    // Build lab summaries
    let mut lab_summaries = Vec::new();

    for lab in labs {
        let lab_record_id = lab
            .id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Lab has no ID"))?;

        // Count nodes in this lab
        let node_count = db::count_nodes_by_lab(&state.db, lab_record_id)
            .await
            .unwrap_or(0);

        lab_summaries.push(LabSummary {
            id: lab.lab_id.clone(),
            name: lab.name.clone(),
            node_count,
            status: LabStatus::Unknown, // Always unknown for now
        });
    }

    let total = lab_summaries.len();

    Ok(ListLabsResponse {
        labs: lab_summaries,
        total,
    })
}
