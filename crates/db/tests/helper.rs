use anyhow::Result;
use data::{NodeConfig, NodeModel};

/// Helper to setup test database connection
pub async fn setup_db() -> Result<surrealdb::Surreal<surrealdb::engine::remote::ws::Client>> {
    let db = db::connect("localhost", 8000, "test", "test_node_configs").await?;
    Ok(db)
}

/// Helper to create a test config
pub fn create_test_config(model: NodeModel) -> NodeConfig {
    NodeConfig::get_model(model)
}
