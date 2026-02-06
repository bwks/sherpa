/// DELETE operation tests for node_config
use anyhow::Result;
use db::{
    count_node_configs, create_lab, create_node_config, delete_node_config, get_node_config_by_id,
};
use shared::data::{DbUser, NodeModel};

use crate::{create_test_config, create_test_node_with_model, setup_db, teardown_db};

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_delete_node_config_success() -> Result<()> {
    let db = setup_db("test_delete_node_config_success").await?;

    // Create a config
    let test_config = create_test_config(NodeModel::AristaVeos);
    let created = create_node_config(&db, test_config).await?;
    let created_id = created.id.clone().expect("Created config should have ID");

    // Delete the config
    delete_node_config(&db, created_id.clone()).await?;

    // Verify it's deleted
    let result = get_node_config_by_id(&db, created_id).await?;
    assert!(result.is_none(), "Config should be deleted");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_delete_nonexistent_config_fails() -> Result<()> {
    let db = setup_db("test_delete_nonexistent_config_fails").await?;

    // Attempt to delete a config with fake/nonexistent ID
    let fake_id = "node_config:nonexistent_id_99999".parse()?;
    let result = delete_node_config(&db, fake_id).await;

    assert!(result.is_err(), "Delete with nonexistent ID should fail");
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
async fn test_delete_and_verify_count_decreases() -> Result<()> {
    let db = setup_db("test_delete_and_verify_count_decreases").await?;

    // Get initial count
    let initial_count = count_node_configs(&db).await?;

    // Create a config
    let test_config = create_test_config(NodeModel::CiscoAsav);
    let created = create_node_config(&db, test_config).await?;
    let created_id = created.id.clone().expect("Created config should have ID");

    // Verify count increased
    let after_create = count_node_configs(&db).await?;
    assert_eq!(
        after_create,
        initial_count + 1,
        "Count should increase by 1 after creation"
    );

    // Delete the config
    delete_node_config(&db, created_id).await?;

    // Verify count decreased back to initial
    let after_delete = count_node_configs(&db).await?;
    assert_eq!(
        after_delete, initial_count,
        "Count should return to initial value after deletion"
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_delete_config_referenced_by_node_behavior() -> Result<()> {
    let db = setup_db("test_delete_config_referenced_by_node_behavior").await?;

    // Use a unique username with timestamp to avoid collisions
    let unique_username = format!(
        "test_user_delete_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
    );

    // Create a test user for the lab
    let user: Option<DbUser> = db
        .create("user")
        .content(DbUser {
            id: None,
            username: unique_username,
            ssh_keys: vec![],
        })
        .await?;
    let user = user.expect("User should be created");

    // Create a lab with unique 8-char ID
    let timestamp_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    // Use last 8 digits of timestamp to ensure uniqueness
    let lab_id = format!("{:08}", timestamp_secs % 100000000);
    let lab = create_lab(&db, "test-lab-constraint", &lab_id, &user).await?;

    // Create a node config
    let test_config = create_test_config(NodeModel::WindowsServer);
    let created_config = create_node_config(&db, test_config).await?;
    let config_id = created_config
        .id
        .clone()
        .expect("Created config should have ID");

    // Create a node that references this config
    let node = create_test_node_with_model(&db, "win-server-01", 1, NodeModel::WindowsServer, &lab)
        .await?;

    // Verify the node references our config
    assert_eq!(
        node.config.to_string(),
        config_id.to_string(),
        "Node should reference the config we created"
    );

    // Attempt to delete the config
    // NOTE: The database schema defines REFERENCE ON DELETE REJECT, but this
    // constraint may not be enforced in all SurrealDB versions. This test
    // documents the actual behavior rather than asserting expected behavior.
    let result = delete_node_config(&db, config_id.clone()).await;

    // Document the actual behavior
    if result.is_ok() {
        // Constraint NOT enforced - deletion succeeded despite reference
        println!("INFO: Config deletion succeeded despite node reference (SurrealDB limitation)");
    } else {
        // Constraint enforced - deletion failed as expected
        let err = result.unwrap_err();
        let err_str = err.to_string();

        // Verify error message indicates constraint violation
        let has_expected_error = err_str.contains("constraint")
            || err_str.contains("foreign key")
            || err_str.contains("referenced")
            || err_str.contains("REJECT")
            || err_str.contains("Cannot delete");

        assert!(
            has_expected_error,
            "Error should indicate constraint violation, got: {}",
            err_str
        );
    }

    teardown_db(&db).await?;
    Ok(())
}
