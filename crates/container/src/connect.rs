use anyhow::Result;
use bollard::Docker;

pub fn docker_connection() -> Result<Docker> {
    let docker = Docker::connect_with_local_defaults()?;
    Ok(docker)
}
