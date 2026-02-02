use anyhow::Result;
use bollard::Docker;
use bollard::models::ContainerSummary;
use bollard::query_parameters::ListContainersOptions;

pub async fn list_containers(docker_conn: &Docker) -> Result<Vec<ContainerSummary>> {
    let options = Some(ListContainersOptions {
        all: true,
        ..Default::default()
    });
    Ok(docker_conn.list_containers(options).await?)
}
