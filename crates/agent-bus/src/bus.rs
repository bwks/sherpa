//! The bus itself: a NATS server with `JetStream`, in a container on the host.
//!
//! It uses the host's network and listens on one address, such as a lab's management bridge, so
//! only that lab's guests and the host reach it. It has no authentication yet.

use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::errors::Error as DockerError;
use bollard::models::{
    ContainerCreateBody, ContainerStateStatusEnum, HostConfig, Mount, MountTypeEnum, RestartPolicy,
    RestartPolicyNameEnum,
};
use bollard::query_parameters::{CreateContainerOptions, CreateImageOptionsBuilder};
use container::{
    container_status, docker_connection, remove_container, start_container, stop_container,
};
use futures_util::TryStreamExt;

/// The bus container.
const CONTAINER_NAME: &str = "sherpa-agent-bus";
/// `nats:2.15-alpine`, nats-server 2.15.2.
const IMAGE: &str = "nats@sha256:afae2fdbe2e326c65e1246cb83b9d05d319188734f3bd577ac8072daae78a10f";
/// The volume holding `JetStream`'s stored messages; it outlives the container.
const VOLUME: &str = "sherpa-agent-bus-data";
const DATA_DIR: &str = "/data";
const PORT: u16 = 4222;
/// The HTTP status Docker answers a create with when the name is taken.
const CONFLICT: u16 = 409;

/// Start the bus listening on `listen`, creating its container the first time.
///
/// # Errors
///
/// Fails when Docker cannot pull the image or create or start the container.
pub async fn start(listen: Ipv4Addr) -> Result<()> {
    let docker = docker_connection()?;
    pull_image(&docker).await?;
    match docker
        .create_container(
            Some(CreateContainerOptions {
                name: Some(CONTAINER_NAME.to_owned()),
                ..Default::default()
            }),
            container_config(listen),
        )
        .await
    {
        Ok(_) => println!("Created {CONTAINER_NAME}"),
        Err(DockerError::DockerResponseServerError { status_code, .. })
            if status_code == CONFLICT =>
        {
            println!("{CONTAINER_NAME} exists; starting it as it was created");
        }
        Err(error) => return Err(error).context("Unable to create the bus container"),
    }
    start_container(&docker, CONTAINER_NAME).await?;
    println!("Bus listening on nats://{listen}:{PORT}");
    Ok(())
}

/// Stop and remove the bus container. The stored messages stay in their volume.
///
/// # Errors
///
/// Fails when Docker cannot stop or remove the container.
pub async fn stop() -> Result<()> {
    let docker = docker_connection()?;
    stop_container(&docker, CONTAINER_NAME).await?;
    remove_container(&docker, CONTAINER_NAME).await?;
    println!("Removed {CONTAINER_NAME}; stored messages are kept in {VOLUME}");
    Ok(())
}

/// Print whether the bus container is running.
///
/// # Errors
///
/// Fails when Docker cannot be reached.
pub async fn status() -> Result<()> {
    let docker = docker_connection()?;
    match container_status(&docker, CONTAINER_NAME).await {
        Ok(Some(ContainerStateStatusEnum::RUNNING)) => println!("{CONTAINER_NAME}: running"),
        Ok(Some(state)) => println!("{CONTAINER_NAME}: {state}"),
        Ok(None) => println!("{CONTAINER_NAME}: state unknown"),
        Err(error) => println!("{CONTAINER_NAME}: not created ({error:#})"),
    }
    Ok(())
}

/// Pull the pinned image unless Docker already has it.
async fn pull_image(docker: &Docker) -> Result<()> {
    if docker.inspect_image(IMAGE).await.is_ok() {
        return Ok(());
    }
    println!("Pulling {IMAGE}");
    let options = CreateImageOptionsBuilder::default()
        .from_image(IMAGE)
        .build();
    let mut stream = docker.create_image(Some(options), None, None);
    while stream
        .try_next()
        .await
        .context("Unable to pull the bus image")?
        .is_some()
    {}
    Ok(())
}

fn container_config(listen: Ipv4Addr) -> ContainerCreateBody {
    ContainerCreateBody {
        image: Some(IMAGE.to_owned()),
        cmd: Some(vec![
            "--addr".to_owned(),
            listen.to_string(),
            "--port".to_owned(),
            PORT.to_string(),
            "--jetstream".to_owned(),
            "--store_dir".to_owned(),
            DATA_DIR.to_owned(),
        ]),
        host_config: Some(HostConfig {
            network_mode: Some("host".to_owned()),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            mounts: Some(vec![Mount {
                target: Some(DATA_DIR.to_owned()),
                source: Some(VOLUME.to_owned()),
                typ: Some(MountTypeEnum::VOLUME),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bus_listens_only_on_the_given_address_with_jetstream() {
        let config = container_config(Ipv4Addr::new(172, 31, 3, 1));
        let command = config.cmd.unwrap();
        assert_eq!(command[..4], ["--addr", "172.31.3.1", "--port", "4222"]);
        assert!(command.contains(&"--jetstream".to_owned()));
        let host = config.host_config.unwrap();
        assert_eq!(host.network_mode.as_deref(), Some("host"));
        assert_eq!(host.mounts.unwrap()[0].source.as_deref(), Some(VOLUME));
    }
}
