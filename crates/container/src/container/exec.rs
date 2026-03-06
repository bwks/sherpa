use anyhow::{Context, Result};
use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecOptions};

/// Execute a command inside a running container in detached mode.
pub async fn exec_container(docker: &Docker, container_name: &str, cmd: Vec<&str>) -> Result<()> {
    let config = CreateExecOptions {
        cmd: Some(cmd),
        user: Some("root"),
        ..Default::default()
    };

    let exec = docker
        .create_exec(container_name, config)
        .await
        .with_context(|| format!("Failed to create exec in container {container_name}"))?;

    docker
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: true,
                ..Default::default()
            }),
        )
        .await
        .with_context(|| format!("Failed to start exec in container {container_name}"))?;

    Ok(())
}
