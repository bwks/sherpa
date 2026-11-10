use anyhow::{Context, Result};
use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_self_or_admin;
use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use shared::data::{DbLab, LabState, LabSummary, ListLabsResponse, NodeState, RecordId};

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
        .ok_or_else(|| ServiceError::NotFound(format!("User not found: {username}")))?;

    // Get user's record ID
    let user_id = user.id.ok_or_else(|| anyhow::anyhow!("User has no ID"))?;

    let labs = db::list_labs_by_user(&state.db, user_id.clone())
        .await
        .context("Failed to list labs for user")?;
    let node_states = db::list_node_states_by_user(&state.db, user_id)
        .await
        .context("Failed to list the nodes of the user's labs")?;
    let labs = summarize_labs(labs, &node_states);
    let total = labs.len();
    Ok(ListLabsResponse { labs, total })
}

/// Summarize each lab: its phase, and its status and node count from `node_states`, the state
/// of every node of these labs with its lab's record ID.
fn summarize_labs(labs: Vec<DbLab>, node_states: &[(RecordId, NodeState)]) -> Vec<LabSummary> {
    labs.into_iter()
        .map(|lab| {
            let states: Vec<NodeState> = node_states
                .iter()
                .filter(|(node_lab, _)| lab.id.as_ref() == Some(node_lab))
                .map(|(_, state)| *state)
                .collect();
            LabSummary {
                id: lab.lab_id,
                name: lab.name,
                description: lab.description,
                node_count: states.len(),
                status: LabState::derive(&states),
                phase: lab.phase,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::LabPhase;

    fn lab(key: &str, phase: LabPhase) -> DbLab {
        DbLab {
            id: Some(RecordId::new("lab", key.to_string())),
            lab_id: key.to_string(),
            name: key.to_string(),
            user: RecordId::new("user", "alice".to_string()),
            loopback_network: String::new(),
            management_network: String::new(),
            gateway_ipv4: String::new(),
            router_ipv4: String::new(),
            tailscale_container_id: None,
            management_network_v6: None,
            gateway_ipv6: None,
            router_ipv6: None,
            loopback_network_v6: None,
            phase,
            client_host: None,
            client_folder: None,
            description: None,
            ssh_key: shared::data::LabSshKey::Lab,
        }
    }

    #[test]
    fn a_labs_status_is_derived_from_its_nodes_when_listed() {
        // #499: the status is no longer stored, so it cannot be stale or missing (M1.7).
        let labs = vec![
            lab("ab12cd34", LabPhase::Ready),
            lab("ef56ab78", LabPhase::Ready),
            lab("empty001", LabPhase::Creating),
        ];
        let node_states = [
            (
                RecordId::new("lab", "ab12cd34".to_string()),
                NodeState::Running,
            ),
            (
                RecordId::new("lab", "ef56ab78".to_string()),
                NodeState::Running,
            ),
            (
                RecordId::new("lab", "ab12cd34".to_string()),
                NodeState::Stopped,
            ),
        ];

        let summaries = summarize_labs(labs, &node_states);

        let shown: Vec<(&str, LabState, usize, LabPhase)> = summaries
            .iter()
            .map(|lab| (lab.id.as_str(), lab.status, lab.node_count, lab.phase))
            .collect();
        assert_eq!(
            shown,
            [
                ("ab12cd34", LabState::Partial, 2, LabPhase::Ready),
                ("ef56ab78", LabState::Running, 1, LabPhase::Ready),
                ("empty001", LabState::Empty, 0, LabPhase::Creating),
            ]
        );
    }
}
