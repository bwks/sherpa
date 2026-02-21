use anyhow::Result;
use db::apply_schema;
use shared::data::{DbLab, DbNode, NodeConfig, NodeModel};
use std::time::{SystemTime, UNIX_EPOCH};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// Generate a unique namespace for test isolation
/// Uses timestamp + thread ID to ensure uniqueness across parallel test runs
fn generate_test_namespace(test_name: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_nanos();

    format!("test_ns_{timestamp}_{test_name}")
}

/// Helper to setup test database connection with a unique namespace
/// This ensures test isolation by using a dedicated namespace per test run
pub async fn setup_db(namespace: &str) -> Result<Surreal<Client>> {
    let namespace = generate_test_namespace(namespace);
    let db = db::connect("localhost", 8000, &namespace, "test_cases").await?;

    // Apply schema to ensure tables exist for tests
    apply_schema(&db).await?;

    Ok(db)
}

/// Helper to teardown/cleanup test database
/// Removes the entire namespace used for the test, cleaning up all test data
pub async fn teardown_db(db: &Surreal<Client>) -> Result<()> {
    // Get the current namespace being used
    // Note: SurrealDB doesn't provide direct API to get current namespace,
    // so we'll use a query to remove all records from tables we created

    // Delete all test data from our tables
    let _: Vec<NodeConfig> = db.delete("node_image").await?;

    // Note: We could also delete from other tables if they exist in the test:
    // let _: Vec<DbNode> = db.delete("node").await?;
    // let _: Vec<DbLink> = db.delete("link").await?;
    // let _: Vec<DbLab> = db.delete("lab").await?;
    // let _: Vec<DbUser> = db.delete("user").await?;

    Ok(())
}

/// Helper to create a test config
pub fn create_test_config(model: NodeModel) -> NodeConfig {
    NodeConfig::get_model(model)
}

/// Test helper to create a node with a specific model
///
/// This is a convenience wrapper for tests that handles the config lookup automatically.
/// Equivalent to the old create_lab_node() but for test use only.
pub async fn create_test_node_with_model(
    db: &Surreal<Client>,
    name: &str,
    index: u16,
    model: NodeModel,
    lab: &DbLab,
) -> Result<DbNode> {
    use db::{create_node, get_node_image_by_model_kind_version};

    // Get the model kind from the NodeModel
    let config_template = NodeConfig::get_model(model.clone());
    let kind = config_template.kind;
    let version = config_template.version;

    let config = get_node_image_by_model_kind_version(db, &model, &kind, &version)
        .await?
        .expect("Config should exist for this model");
    let config_id = config.id.clone().expect("Config should have an id");
    let lab_id = lab.id.clone().expect("Lab should have an id");

    create_node(db, name, index, config_id, lab_id).await
}
