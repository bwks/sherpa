/// CREATE operation tests for node_config
use anyhow::Result;
use db::{create_node_config, list_node_configs, upsert_node_config};
use shared::data::NodeModel;

use crate::{create_test_config, setup_db, teardown_db};

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_create_and_list_node_configs() -> Result<()> {
    let db = setup_db("test_create_and_list_node_configs").await?;

    // Create a test config
    let test_config = create_test_config(NodeModel::AristaVeos);
    let created = create_node_config(&db, test_config.clone()).await?;

    assert_eq!(created.model, NodeModel::AristaVeos);
    assert!(created.id.is_some(), "Created config should have an ID");

    // List all configs
    let all_configs = list_node_configs(&db).await?;
    assert!(
        !all_configs.is_empty(),
        "Should have at least one config after creation"
    );

    // Verify our config is in the list
    let found = all_configs.iter().any(|c| c.model == NodeModel::AristaVeos);
    assert!(found, "Created config should be in the list");

    // Cleanup
    teardown_db(&db).await?;

    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_upsert_node_config() -> Result<()> {
    let db = setup_db("test_upsert_node_config").await?;

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

    // Cleanup
    teardown_db(&db).await?;

    Ok(())
}
