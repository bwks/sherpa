use anyhow::{Context, Result};

use shared::data::{ContainerPullRequest, ContainerPullResponse};

use crate::daemon::state::AppState;

/// Pull a container image from an OCI registry via Docker
pub async fn pull_container_image(
    request: ContainerPullRequest,
    _state: &AppState,
) -> Result<ContainerPullResponse> {
    tracing::info!(
        repo = %request.repo,
        tag = %request.tag,
        "Pulling container image"
    );

    container::pull_image(&request.repo, &request.tag)
        .await
        .context(format!(
            "Failed to pull container image {}:{}",
            request.repo, request.tag
        ))?;

    Ok(ContainerPullResponse {
        success: true,
        repo: request.repo,
        tag: request.tag,
        message: "Image pulled successfully".to_string(),
    })
}
