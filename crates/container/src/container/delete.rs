use anyhow::{Context, Result};
use bollard::Docker;
use bollard::query_parameters::{KillContainerOptions, RemoveContainerOptions};

pub async fn kill_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .kill_container(
            name,
            Some(KillContainerOptions {
                signal: "SIGKILL".to_string(),
            }),
        )
        .await
        .with_context(|| format!("Error destroying container: {name}"))?;

    println!("Destroyed container: {name}");
    Ok(())
}

pub async fn remove_container(docker: &Docker, name: &str) -> Result<()> {
    // Wait for the container to exit, then remove (emulates --rm)
    docker
        .remove_container(
            name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await?;
    Ok(())
}
