use anyhow::{Context, Result};
use opentelemetry::KeyValue;
use std::time::Instant;

use shared::data::{ContainerPullRequest, ContainerPullResponse, NodeConfig, NodeKind, StatusKind};

use tracing::instrument;

use crate::daemon::state::AppState;
use crate::services::progress::ProgressSender;

/// Pull a container image from an OCI registry via Docker and import to DB
#[instrument(skip(state, progress), fields(model = %request.model, repo = %request.repo, tag = %request.tag))]
pub async fn pull_container_image(
    request: ContainerPullRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<ContainerPullResponse> {
    let start = Instant::now();

    tracing::info!(
        model = %request.model,
        repo = %request.repo,
        tag = %request.tag,
        "Pulling container image"
    );

    container::pull_image(&request.repo, &request.tag, |msg| {
        let _ = progress.send_status(msg.to_string(), StatusKind::Progress);
    })
    .await
    .context(format!(
        "Failed to pull container image {}:{}",
        request.repo, request.tag
    ))?;

    let _ = progress.send_status("Updating database...".to_string(), StatusKind::Progress);

    // Import pulled image to database
    let existing_versions =
        db::get_node_image_versions(&state.db, &request.model, &NodeKind::Container).await?;
    let make_default = if existing_versions.is_empty() {
        true
    } else {
        request.default
    };
    let db_config = pulled_node_image(&request, make_default);

    let db_tracked = match db::upsert_node_image(&state.db, db_config).await {
        Ok(_) => {
            tracing::info!(
                "Upserted node_image for model={} version={}",
                request.model,
                request.tag
            );
            true
        }
        Err(e) => {
            tracing::error!(
                "Failed to upsert node_image for model={} version={}: {:?}",
                request.model,
                request.tag,
                e
            );
            false
        }
    };

    let _ = progress.send_status(
        format!(
            "Container image {}:{} pulled successfully",
            request.repo, request.tag
        ),
        StatusKind::Done,
    );

    state.metrics.operation_duration.record(
        start.elapsed().as_secs_f64(),
        &[KeyValue::new("operation.type", "container_pull")],
    );

    Ok(ContainerPullResponse {
        success: true,
        model: request.model.to_string(),
        repo: request.repo,
        tag: request.tag,
        db_tracked,
        message: "Image pulled successfully".to_string(),
    })
}

/// The node image record for a pulled container image: the model's defaults, with the version
/// and repo it was pulled from. Models without a built-in repo run the repo recorded here.
fn pulled_node_image(request: &ContainerPullRequest, default: bool) -> NodeConfig {
    let mut config = NodeConfig::get_model(request.model);
    config.version = request.tag.clone();
    config.repo = Some(request.repo.clone());
    config.default = default;
    config.id = None;
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeModel;

    #[test]
    fn test_pulled_node_image_records_the_repo_it_was_pulled_from() {
        let request = ContainerPullRequest {
            model: NodeModel::PostgresqlDb,
            repo: "postgres".to_string(),
            tag: "17".to_string(),
            default: false,
        };

        let config = pulled_node_image(&request, true);

        assert_eq!(config.model, NodeModel::PostgresqlDb);
        assert_eq!(config.repo.as_deref(), Some("postgres"));
        assert_eq!(config.version, "17");
        assert!(config.default);
        assert_eq!(config.container_repo(None).as_deref(), Some("postgres"));
    }
}
