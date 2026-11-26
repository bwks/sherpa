//! The Docker of a node VM: a Flatcar VM that runs one container node, so the privileges the
//! node's image needs reach that VM's kernel and not the server's. sherpad drives it over vsock
//! with mutual TLS, copies the node's image and files into it, and runs the container with the
//! settings a container on the server gets.

mod tar;
mod transport;

use std::sync::{Arc, Mutex, PoisonError};

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::body_full;
use bollard::errors::Error as DockerError;
use bollard::query_parameters::{
    CreateContainerOptions, ImportImageOptionsBuilder, UploadToContainerOptions,
};
use futures_util::StreamExt;
use futures_util::future;
use tracing::instrument;

use shared::data::Dns;

use crate::container::{ContainerSpec, node_vm_create_body, remove_container, start_and_attach};
use crate::status::is_missing;

pub use tar::NodeVmFile;
pub use transport::{NodeVmTls, node_vm_docker};

/// Copy image `image` from the server's Docker into the node VM's, unless the VM has it already.
/// Docker streams the image out and the VM's Docker loads it as it arrives. Returns whether it
/// was copied.
///
/// # Errors
///
/// Fails when the server's Docker cannot export the image or the VM's cannot load it.
#[instrument(skip(host, vm), level = "debug")]
pub async fn copy_image_to_node_vm(host: &Docker, vm: &Docker, image: &str) -> Result<bool> {
    match vm.inspect_image(image).await {
        Ok(_) => return Ok(false),
        Err(error) if is_missing(&error) => {}
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot look for image {image} in the VM"));
        }
    }
    tracing::info!(%image, "Copying the image into the node VM");
    // The VM takes the image as a stream of plain chunks, so an export error ends the stream
    // and is kept here to be reported.
    let export_error = Arc::new(Mutex::new(None::<DockerError>));
    let stream_error = Arc::clone(&export_error);
    let chunks = host.export_image(image).scan((), move |(), chunk| {
        future::ready(match chunk {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                *stream_error.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                None
            }
        })
    });
    let options = ImportImageOptionsBuilder::default().quiet(true).build();
    let mut loading = vm.import_image_stream(options, chunks, None);
    let mut load_error = None;
    while let Some(result) = loading.next().await {
        if let Err(error) = result {
            load_error = Some(error);
            break;
        }
    }
    if let Some(error) = export_error
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    {
        return Err(error).with_context(|| format!("Cannot export image {image}"));
    }
    if let Some(error) = load_error {
        return Err(error).with_context(|| format!("The node VM cannot load image {image}"));
    }
    Ok(true)
}

/// Create the node's container in its VM from `spec`, in the VM's network namespace with the
/// lab's `dns`, copy `files` into it, and start it. A container of that name already there is
/// replaced. Returns whether it runs; one that cannot be set up is removed again.
///
/// # Errors
///
/// Fails when a file path is not absolute, or the VM's Docker cannot create, fill, start or
/// inspect the container.
#[instrument(skip(vm, spec, dns, files), fields(container_name = %spec.name), level = "debug")]
pub async fn run_node_vm_container(
    vm: &Docker,
    spec: ContainerSpec,
    dns: &Dns,
    files: &[NodeVmFile],
) -> Result<bool> {
    let name = spec.name.clone();
    let archive = tar::archive(files)?;
    remove_container(vm, &name).await?;
    let body = node_vm_create_body(spec, dns)
        .with_context(|| format!("Invalid resource limits for container {name}"))?;
    let created = vm
        .create_container(
            Some(CreateContainerOptions {
                name: Some(name.clone()),
                ..Default::default()
            }),
            body,
        )
        .await
        .with_context(|| format!("Cannot create container {name} in its node VM"))?;
    let started = async {
        vm.upload_to_container(
            &created.id,
            Some(UploadToContainerOptions {
                path: "/".to_owned(),
                ..Default::default()
            }),
            body_full(archive.into()),
        )
        .await
        .with_context(|| format!("Cannot copy the node's files into container {name}"))?;
        start_and_attach(vm, &created.id, &name, &[]).await
    }
    .await;
    if started.is_err()
        && let Err(removal) = remove_container(vm, &created.id).await
    {
        tracing::warn!(container_name = %name, error = %removal, "Cannot remove a node VM container that could not be set up");
    }
    started
}
