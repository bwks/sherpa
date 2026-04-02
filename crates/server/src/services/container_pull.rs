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
    let config = NodeConfig::get_model(request.model);

    let existing_versions =
        db::get_node_image_versions(&state.db, &request.model, &NodeKind::Container).await?;
    let make_default = if existing_versions.is_empty() {
        true
    } else {
        request.default
    };

    let mut db_config = config;
    db_config.version = request.tag.clone();
    db_config.default = make_default;
    db_config.id = None;

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
