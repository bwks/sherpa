use anyhow::Result;
use db::{Database, LabNetworks, apply_schema};
use shared::data::{DbLab, DbNode, NodeConfig, NodeModel};
use shared::konst::SHERPA_PASSWORD;
use std::time::{SystemTime, UNIX_EPOCH};

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
pub async fn setup_db(namespace: &str) -> Result<Database> {
    let namespace = generate_test_namespace(namespace);
    let db_password =
        std::env::var("SHERPA_DB_PASSWORD").unwrap_or_else(|_| SHERPA_PASSWORD.to_string());
    let db_port: u16 = std::env::var("SHERPA_DEV_DB_PORT")
        .unwrap_or_else(|_| "42069".to_string())
        .parse()
        .expect("SHERPA_DEV_DB_PORT must be a valid port number");
    let db = db::connect("localhost", db_port, &namespace, "test_cases", &db_password).await?;

    // Apply schema to ensure tables exist for tests
    apply_schema(&db).await?;

    Ok(db)
}

/// Helper to teardown/cleanup test database
/// Removes the entire namespace used for the test, cleaning up all test data
pub async fn teardown_db(db: &Database) -> Result<()> {
    // Get the current namespace being used
    // Note: SurrealDB doesn't provide direct API to get current namespace,
    // so we'll use a query to remove all records from tables we created

    // Delete all test data in dependency order (children before parents)
    db.query("DELETE link").await?;
    db.query("DELETE bridge").await?;
    db.query("DELETE node").await?;
    db.query("DELETE lab").await?;
    db.query("DELETE node_image").await?;
    db.query("DELETE user").await?;

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
    db: &Database,
    name: &str,
    index: u16,
    model: NodeModel,
    lab: &DbLab,
) -> Result<DbNode> {
    use db::{create_node, get_node_image_by_model_kind_version};

    // Get the model kind from the NodeModel
    let config_template = NodeConfig::get_model(model);
    let kind = config_template.kind;
    let version = config_template.version;

    let config = get_node_image_by_model_kind_version(db, &model, &kind, &version)
        .await?
        .expect("Config should exist for this model");
    let config_id = config.id.clone().expect("Config should have an id");
    let lab_id = lab.id.clone().expect("Lab should have an id");

    create_node(db, name, index, config_id, lab_id).await
}

/// IPv4-only lab networks for tests that do not exercise IPv6 allocation.
pub fn ipv4_lab_networks(
    loopback_network: &str,
    management_network: &str,
    gateway_ipv4: &str,
    router_ipv4: &str,
) -> LabNetworks {
    LabNetworks {
        loopback_network: loopback_network.to_string(),
        management_network: management_network.to_string(),
        gateway_ipv4: gateway_ipv4.to_string(),
        router_ipv4: router_ipv4.to_string(),
        management_network_v6: None,
        loopback_network_v6: None,
        gateway_ipv6: None,
        router_ipv6: None,
    }
}
