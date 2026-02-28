/// UPDATE operation tests for node_image
use anyhow::Result;
use db::{create_node_image, get_node_image_by_id, update_node_image};
use shared::data::{NodeModel, RecordId};

use crate::{create_test_config, setup_db, teardown_db};

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_update_node_image_success() -> Result<()> {
    let db = setup_db("test_update_node_image_success").await?;

    // Create a config
    let test_config = create_test_config(NodeModel::AristaVeos);
    let created = create_node_image(&db, test_config).await?;
    let original_id = created.id.clone();

    // Modify multiple fields
    let mut updated_config = created.clone();
    updated_config.memory = 4096;
    updated_config.cpu_count = 4;
    updated_config.data_interface_count = 24;

    // Update the config
    let result = update_node_image(&db, updated_config).await?;

    // Verify changes were applied
    assert_eq!(result.memory, 4096, "Memory should be updated");
    assert_eq!(result.cpu_count, 4, "CPU count should be updated");
    assert_eq!(
        result.data_interface_count, 24,
        "Interface count should be updated"
    );
    assert_eq!(result.id, original_id, "ID should remain unchanged");
    assert_eq!(result.model, NodeModel::AristaVeos, "Model unchanged");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_update_node_image_without_id_fails() -> Result<()> {
    let db = setup_db("test_update_node_image_without_id_fails").await?;

    // Create a config without an ID
    let mut test_config = create_test_config(NodeModel::CiscoAsav);
    test_config.id = None; // Explicitly remove ID

    // Attempt to update should fail
    let result = update_node_image(&db, test_config).await;

    assert!(result.is_err(), "Update without ID should fail");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("has no ID"),
        "Error should mention missing ID: {}",
        err
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_update_nonexistent_config_fails() -> Result<()> {
    let db = setup_db("test_update_nonexistent_config_fails").await?;

    // Create a config with a fake/nonexistent ID
    let mut test_config = create_test_config(NodeModel::JuniperVrouter);
    test_config.id = Some(RecordId::new("node_image", "nonexistent_id_12345"));

    // Attempt to update should fail
    let result = update_node_image(&db, test_config).await;

    assert!(result.is_err(), "Update with nonexistent ID should fail");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("not found"),
        "Error should mention record not found: {}",
        err
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_update_preserves_id() -> Result<()> {
    let db = setup_db("test_update_preserves_id").await?;

    // Create a config
    let test_config = create_test_config(NodeModel::NokiaSrlinux);
    let created = create_node_image(&db, test_config).await?;
    let original_id = created.id.clone().expect("Created config should have ID");

    // Update multiple times
    for i in 1..=3 {
        let mut updated_config = created.clone();
        updated_config.memory = 1024 * i;
        updated_config.id = original_id.clone().into();

        let result = update_node_image(&db, updated_config).await?;

        assert_eq!(
            result.id,
            original_id.clone().into(),
            "ID should remain constant after update {i}"
        );
        assert_eq!(
            result.memory,
            1024 * i,
            "Memory should be updated to {}",
            1024 * i
        );
    }

    // Verify via get_by_id that ID is still correct
    let retrieved = get_node_image_by_id(&db, original_id.clone())
        .await?
        .expect("Config should still exist");

    assert_eq!(
        retrieved.id,
        original_id.into(),
        "Retrieved config should have original ID"
    );
    assert_eq!(retrieved.memory, 3072, "Final memory value should be 3072");

    teardown_db(&db).await?;
    Ok(())
}
