/// READ operation tests for node_config
use anyhow::Result;
use db::{
    count_node_configs, create_node_config, get_node_config_by_id, get_node_config_by_model_kind,
    list_node_configs,
};
use shared::data::NodeModel;

use crate::{create_test_config, setup_db, teardown_db};

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_get_node_config_by_id() -> Result<()> {
    let db = setup_db("test_get_node_config_by_id").await?;

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

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_get_node_config_by_model_kind() -> Result<()> {
    let db = setup_db("test_get_node_config_by_model_kind").await?;

    // Create a test config
    let test_config = create_test_config(NodeModel::CiscoAsav);
    let _created = create_node_config(&db, test_config.clone()).await?;

    // Get by model and kind
    let retrieved =
        get_node_config_by_model_kind(&db, &NodeModel::CiscoAsav, &test_config.kind.to_string())
            .await?;

    assert!(retrieved.is_some(), "Should find config by model and kind");
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.model, NodeModel::CiscoAsav);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_count_node_configs() -> Result<()> {
    let db = setup_db("test_count_node_configs").await?;

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

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_get_nonexistent_config() -> Result<()> {
    let db = setup_db("test_get_nonexistent_config").await?;

    // Try to get a config that doesn't exist
    let result =
        get_node_config_by_model_kind(&db, &NodeModel::WindowsServer, "virtual_machine").await?;

    // Should return None if not found
    assert!(
        result.is_none(),
        "Should return None for nonexistent config"
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_list_configs_returns_all_fields() -> Result<()> {
    let db = setup_db("test_list_configs_returns_all_fields").await?;

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
    assert!(ubuntu_config.data_interface_count > 0);
    assert!(!ubuntu_config.interface_prefix.is_empty());

    teardown_db(&db).await?;
    Ok(())
}
