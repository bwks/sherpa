use std::io;

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::body_try_stream;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{CreateContainerOptions, UploadToContainerOptions};
use futures_util::TryStreamExt;
use tracing::instrument;

use crate::container::remove_container;
use crate::error::ContainerError;

/// Where the target directory is bind-mounted in the unpack container.
const UNPACK_MOUNT: &str = "/.sherpa-rootfs";
/// The command of the unpack containers, which are never started: Docker refuses to create a
/// container of an image with no command, such as cEOS, without one.
const UNPACK_COMMAND: &str = "/.sherpa-never-started";

/// What an image runs and with which environment, from its config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageRunConfig {
    pub id: String,
    pub entrypoint: Vec<String>,
    pub cmd: Vec<String>,
    pub env: Vec<String>,
    pub working_dir: String,
}

/// Read the ID, entrypoint, command, environment and working directory of image `reference`.
///
/// # Errors
///
/// Fails when Docker cannot inspect the image, or it has no ID.
#[instrument(skip(docker), level = "debug")]
pub async fn image_run_config(docker: &Docker, reference: &str) -> Result<ImageRunConfig> {
    let image = docker
        .inspect_image(reference)
        .await
        .with_context(|| format!("Failed to inspect container image {reference}"))?;
    let id = image.id.ok_or_else(|| ContainerError::ImageWithoutId {
        reference: reference.to_string(),
    })?;
    let config = image.config.unwrap_or_default();
    Ok(ImageRunConfig {
        id,
        entrypoint: config.entrypoint.unwrap_or_default(),
        cmd: config.cmd.unwrap_or_default(),
        env: config.env.unwrap_or_default(),
        working_dir: config.working_dir.unwrap_or_default(),
    })
}

/// Unpack the filesystem of image `reference` into host directory `target`, as root, through
/// Docker: an unstarted container of the image, `<helper_name>-src`, is exported, and the
/// archive uploaded into a second unstarted container, `helper_name`, that bind-mounts
/// `target`. Ownership, modes and file capabilities are kept. Both containers are removed
/// afterwards, whether the unpack worked or not.
///
/// # Errors
///
/// Fails when Docker cannot create the containers, export the image's or take the upload.
#[instrument(skip(docker), level = "debug")]
pub async fn unpack_image_rootfs(
    docker: &Docker,
    reference: &str,
    target: &str,
    helper_name: &str,
) -> Result<()> {
    let source_name = format!("{helper_name}-src");
    let unpacked = async {
        create_unpack_container(docker, reference, &source_name, None).await?;
        create_unpack_container(
            docker,
            reference,
            helper_name,
            Some(format!("{target}:{UNPACK_MOUNT}")),
        )
        .await?;
        copy_rootfs(docker, &source_name, helper_name).await
    }
    .await;
    for name in [source_name.as_str(), helper_name] {
        if let Err(error) = remove_container(docker, name).await {
            tracing::warn!(%name, %error, "Failed to remove an unpack container");
        }
    }
    unpacked.with_context(|| format!("Failed to unpack {reference} into {target}"))
}

/// Create unstarted, networkless container `name` of image `reference`, with bind mount
/// `bind` when given.
async fn create_unpack_container(
    docker: &Docker,
    reference: &str,
    name: &str,
    bind: Option<String>,
) -> Result<()> {
    let body = ContainerCreateBody {
        image: Some(reference.to_string()),
        cmd: Some(vec![UNPACK_COMMAND.to_string()]),
        network_disabled: Some(true),
        host_config: Some(HostConfig {
            binds: bind.map(|bind| vec![bind]),
            network_mode: Some("none".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    docker
        .create_container(
            Some(CreateContainerOptions {
                name: Some(name.to_string()),
                ..Default::default()
            }),
            body,
        )
        .await
        .with_context(|| format!("Failed to create the unpack container {name} for {reference}"))?;
    Ok(())
}

/// Stream the export of container `source` into the unpack mount of container `helper`.
async fn copy_rootfs(docker: &Docker, source: &str, helper: &str) -> Result<()> {
    let export = docker
        .export_container(source)
        .map_err(|error| io::Error::other(error.to_string()));
    docker
        .upload_to_container(
            helper,
            Some(UploadToContainerOptions {
                path: UNPACK_MOUNT.to_string(),
                ..Default::default()
            }),
            body_try_stream(export),
        )
        .await
        .context("Docker did not take the image's filesystem")
}
