use anyhow::{Context, Result, bail};
use opentelemetry::KeyValue;
use std::time::Instant;

use shared::data::{
    ContainerPullRequest, ContainerPullResponse, ContainerSettings, KindSettings, ModelKind,
    NodeConfig, NodeKind, StatusKind, ZtpSettings,
};

use tracing::instrument;

use db::DefaultRequest;

use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use crate::services::image_lifecycle::{image_model_kind, lock_image};
use crate::services::progress::ProgressSender;
use crate::validate::validate_image_version;

/// Pull a container image from an OCI registry via Docker and import to DB
#[instrument(skip(state, progress), fields(model = %request.model, repo = ?request.repo, tag = %request.tag))]
pub async fn pull_container_image(
    request: ContainerPullRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<ContainerPullResponse> {
    let start = Instant::now();
    validate_image_version(&request.tag)?;
    let model_kind = image_model_kind(request.model, request.kind)?;
    if model_kind.kind() != NodeKind::Container {
        return Err(ServiceError::Invalid(format!(
            "'{}' is a {}: only a container image is pulled from a registry",
            request.model,
            model_kind.kind()
        ))
        .into());
    }
    let repo = pull_repo(model_kind, request.repo.as_deref())?;
    let _image_guard = lock_image(model_kind, &request.tag).await;

    tracing::info!(
        model = %request.model,
        %repo,
        tag = %request.tag,
        "Pulling container image"
    );

    container::pull_image(&repo, &request.tag, |msg| {
        let _ = progress.send_status(msg.to_string(), StatusKind::Progress);
    })
    .await
    .with_context(|| format!("Failed to pull container image {repo}:{}", request.tag))?;

    let _ = progress.send_status("Updating database...".to_string(), StatusKind::Progress);

    let request_default = if request.default {
        DefaultRequest::Make
    } else {
        DefaultRequest::Keep
    };
    let recorded = db::upsert_node_image(
        &state.db,
        pulled_node_image(model_kind, &repo, &request.tag)?,
        request_default,
    )
    .await
    .with_context(|| {
        format!(
            "Pulled {repo}:{} but failed to record it in the database; pull again or run \
                 'sherpa server image scan'",
            request.tag
        )
    })?;
    // A version pulled before from another repo runs from the repo pulled now.
    if recorded.container_repo(None).as_deref() != Some(repo.as_str()) {
        let repulled = NodeConfig {
            settings: KindSettings::Container(ContainerSettings {
                repo: Some(repo.clone()),
                ztp: recorded
                    .container()
                    .map_or_else(ZtpSettings::default, |container| container.ztp.clone()),
            }),
            ..recorded
        };
        db::update_node_image(&state.db, repulled, DefaultRequest::Keep)
            .await
            .context("Failed to record the repo the image was pulled from")?;
    }
    tracing::info!(
        model = %request.model,
        version = %request.tag,
        "Recorded the pulled container image"
    );

    let _ = progress.send_status(
        format!("Container image {repo}:{} pulled successfully", request.tag),
        StatusKind::Done,
    );

    state.metrics.operation_duration.record(
        start.elapsed().as_secs_f64(),
        &[KeyValue::new("operation.type", "container_pull")],
    );

    Ok(ContainerPullResponse {
        success: true,
        model: request.model.to_string(),
        repo,
        tag: request.tag,
        db_tracked: true,
        message: "Image pulled successfully".to_string(),
    })
}

/// The repo a pull of `model_kind` is from: `requested`, or else the model's built-in repo.
///
/// # Errors
///
/// Refuses a pull with no repo for a model that has no built-in repo, naming the model.
fn pull_repo(model_kind: ModelKind, requested: Option<&str>) -> Result<String> {
    if let Some(repo) = requested {
        return Ok(repo.to_string());
    }
    model_kind
        .node_config()
        .container_repo(None)
        .ok_or_else(|| {
            ServiceError::Invalid(format!(
                "'{}' has no built-in repo: pass --repo with the image reference and its tag \
             (e.g. nats:2.11 or registry.io/account/image:version)",
                model_kind.model()
            ))
            .into()
        })
}

/// The node image record for a pulled container image: the model's defaults, with the version
/// (`tag`) and `repo` it was pulled from. A container node runs the repo recorded here unless
/// its manifest names an `image`.
///
/// # Errors
///
/// Fails when `model_kind` is not a container's.
fn pulled_node_image(model_kind: ModelKind, repo: &str, tag: &str) -> Result<NodeConfig> {
    let config = model_kind.node_config();
    let KindSettings::Container(container) = config.settings else {
        bail!("{model_kind} is not a container: only a container image is pulled");
    };
    Ok(NodeConfig {
        id: None,
        version: tag.to_string(),
        settings: KindSettings::Container(ContainerSettings {
            repo: Some(repo.to_string()),
            ..container
        }),
        ..config
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeModel;

    fn container(model: NodeModel) -> ModelKind {
        ModelKind::new(model, Some(NodeKind::Container)).unwrap()
    }

    #[test]
    fn image_pull_002_the_repo_is_recorded_as_it_was_written() {
        let config = pulled_node_image(
            ModelKind::default_for(NodeModel::PostgresqlDb),
            "postgres",
            "17",
        )
        .unwrap();

        assert_eq!(config.model, NodeModel::PostgresqlDb);
        assert_eq!(config.version, "17");
        assert_eq!(config.container_repo(None).as_deref(), Some("postgres"));
    }

    #[test]
    fn image_pull_009_without_a_repo_a_model_is_pulled_from_the_servers_built_in_repo() {
        let repo = pull_repo(container(NodeModel::NatsBus), None).unwrap();

        assert_eq!(repo, "nats");
    }

    #[test]
    fn image_pull_010_without_a_repo_a_model_with_no_built_in_repo_is_refused_naming_it() {
        let error = pull_repo(container(NodeModel::Generic), None).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ServiceError>(),
                Some(ServiceError::Invalid(_))
            ),
            "{error:#}"
        );
        let message = error.to_string();
        assert!(message.contains("'generic'"), "{message}");
        assert!(message.contains("--repo"), "{message}");
    }
}
