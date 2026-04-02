use anyhow::Result;
use bollard::Docker;
use tracing::instrument;

#[instrument(level = "debug")]
pub fn docker_connection() -> Result<Docker> {
    let docker = Docker::connect_with_local_defaults()?;
    Ok(docker)
}
