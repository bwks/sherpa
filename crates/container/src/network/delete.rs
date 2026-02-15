use anyhow::Result;
use bollard::Docker;

pub async fn delete_network(docker: &Docker, name: &str) -> Result<()> {
    match docker.remove_network(name).await {
        Ok(_) => tracing::info!(network_name = %name, "Destroyed container network"),
        Err(e) => {
            tracing::error!(network_name = %name, error = %e, "Error deleting container network")
        }
    }

    Ok(())
}
