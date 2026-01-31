/// Integration tests for node_config CRUD operations
/// 
/// These tests require a running SurrealDB instance.
/// Run: surreal start --log trace --user sherpa --pass 'Everest1953!' memory
/// 
/// To run these tests: cargo test --package db --test node_config_tests
use anyhow::Result;
use data::{NodeConfig, NodeModel};
use db::{
    connect, count_node_configs, create_node_config, get_node_config_by_id,
    get_node_config_by_model_kind, list_node_configs, upsert_node_config,
};

/// Helper to setup test database connection
async fn setup_db() -> Result<surrealdb::Surreal<surrealdb::engine::remote::ws::Client>> {
    let db = connect("localhost", 8000, "test", "test_node_configs").await?;
    Ok(db)
}

/// Helper to create a test config
fn create_test_config(model: NodeModel) -> NodeConfig {
    NodeConfig::get_model(model)
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_create_and_list_node_configs() -> Result<()> {
    let db = setup_db().await?;

    // Create a test config
    let test_config = create_test_config(NodeModel::AristaVeos);
    let created = create_node_config(&db, test_config.clone()).await?;

    assert_eq!(created.model, NodeModel::AristaVeos);
    assert!(created.id.is_some(), "Created config should have an ID");

    // List all configs
    let all_configs = list_node_configs(&db).await?;
    assert!(
        all_configs.len() > 0,
        "Should have at least one config after creation"
    );

    // Verify our config is in the list
    let found = all_configs.iter().any(|c| c.model == NodeModel::AristaVeos);
    assert!(found, "Created config should be in the list");

    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_get_node_config_by_id() -> Result<()> {
    let db = setup_db().await?;

    // Create a test config
    let test_config = create_test_config(NodeModel::AristaCeos);
    let created = create_node_config(&db, test_config).await?;

    let created_id = created
        .id
        .clone()
        .expect("Created config should have an ID");

    // Get by ID
    let retrieved = get_node_config_by_id(&db, created_id).await?;

    assert!(retrieved.is_some(), "Should find config by ID");
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.model, NodeModel::AristaCeos);

    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_get_node_config_by_model_kind() -> Result<()> {
    let db = setup_db().await?;

    // Create a test config
    let test_config = create_test_config(NodeModel::CiscoAsav);
    let _created = create_node_config(&db, test_config.clone()).await?;

    // Get by model and kind
    let retrieved = get_node_config_by_model_kind(
        &db,
        &NodeModel::CiscoAsav,
        &test_config.kind.to_string(),
    )
    .await?;

    assert!(
        retrieved.is_some(),
        "Should find config by model and kind"
    );
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.model, NodeModel::CiscoAsav);

    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_count_node_configs() -> Result<()> {
    let db = setup_db().await?;

    let initial_count = count_node_configs(&db).await?;

    // Create a test config
    let test_config = create_test_config(NodeModel::JuniperVrouter);
    let _created = create_node_config(&db, test_config).await?;

    let new_count = count_node_configs(&db).await?;
    assert_eq!(
        new_count,
        initial_count + 1,
        "Count should increase by 1 after creation"
    );

    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_upsert_node_config() -> Result<()> {
    let db = setup_db().await?;

    // First upsert - should create
    let test_config = create_test_config(NodeModel::NokiaSrlinux);
    let first_upsert = upsert_node_config(&db, test_config.clone()).await?;

    assert_eq!(first_upsert.model, NodeModel::NokiaSrlinux);
    let first_id = first_upsert.id.clone();

    // Second upsert - should return existing
    let second_upsert = upsert_node_config(&db, test_config).await?;

    assert_eq!(second_upsert.model, NodeModel::NokiaSrlinux);
    assert_eq!(
        second_upsert.id, first_id,
        "Upsert should return the same record"
    );

    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_get_nonexistent_config() -> Result<()> {
    let db = setup_db().await?;

    // Try to get a config that doesn't exist
    let result =
        get_node_config_by_model_kind(&db, &NodeModel::WindowsServer, "virtual_machine").await?;

    // Should return None if not found
    assert!(
        result.is_none(),
        "Should return None for nonexistent config"
    );

    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_list_configs_returns_all_fields() -> Result<()> {
    let db = setup_db().await?;

    // Create a config
    let test_config = create_test_config(NodeModel::UbuntuLinux);
    let _created = create_node_config(&db, test_config).await?;

    // List and verify all fields are populated
    let configs = list_node_configs(&db).await?;
    let ubuntu_config = configs
        .iter()
        .find(|c| c.model == NodeModel::UbuntuLinux)
        .expect("Should find Ubuntu config");

    // Verify key fields
    assert!(ubuntu_config.id.is_some());
    assert!(ubuntu_config.cpu_count > 0);
    assert!(ubuntu_config.memory > 0);
    assert!(ubuntu_config.interface_count > 0);
    assert!(!ubuntu_config.interface_prefix.is_empty());

    Ok(())
}
