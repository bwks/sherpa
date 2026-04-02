use anyhow::Result;
use bollard::Docker;
use bollard::models::Network;
use bollard::query_parameters::ListNetworksOptions;
use tracing::instrument;

#[instrument(skip(docker_conn), level = "debug")]
pub async fn list_networks(docker_conn: &Docker) -> Result<Vec<Network>> {
    let options = Some(ListNetworksOptions {
        ..Default::default()
    });
    Ok(docker_conn.list_networks(options).await?)
}
