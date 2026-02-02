use anyhow::Result;
use bollard::Docker;

pub async fn delete_network(docker: &Docker, name: &str) -> Result<()> {
    match docker.remove_network(name).await {
        Ok(_) => println!("Destroyed container network: {}", name),
        Err(e) => eprintln!("Error deleting container network: {}", e),
    }

    Ok(())
}
