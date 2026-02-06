use anyhow::Result;
use db::{create_lab, create_node, create_node_config, create_user, get_node, update_node};
use shared::data::{NodeConfig, NodeModel};

use crate::helper::{setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_update_node_success() -> Result<()> {
    let db = setup_db("test_update_node").await?;

    let user = create_user(&db, "alice".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;
    let config1 =
        create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;
    let config2 =
        create_node_config(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    // Create node
    let mut node = create_node(
        &db,
        "node1",
        1,
        config1.id.unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Update node
    node.name = "updated-node".to_string();
    node.index = 10;
    let config2_id = config2.id.clone().unwrap();
    node.config = config2_id.clone();

    let updated = update_node(&db, node).await?;

    assert_eq!(updated.name, "updated-node");
    assert_eq!(updated.index, 10);
    assert_eq!(updated.config, config2_id);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_node_without_id_fails() -> Result<()> {
    let db = setup_db("test_update_node_no_id").await?;

    let user = create_user(&db, "bob".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Lab", "lab-0002", &user).await?;
    let config =
        create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create node without ID
    let mut node = create_node(
        &db,
        "node1",
        1,
        config.id.unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Remove ID and try to update
    node.id = None;

    let result = update_node(&db, node).await;

    assert!(result.is_err(), "Should fail when ID is None");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("id") || error_msg.contains("ID"),
        "Error should mention missing ID, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_node_change_lab_fails() -> Result<()> {
    let db = setup_db("test_update_node_change_lab").await?;

    let user = create_user(&db, "charlie".to_string(), vec![]).await?;
    let lab1 = create_lab(&db, "Lab 1", "lab-0003", &user).await?;
    let lab2 = create_lab(&db, "Lab 2", "lab-0004", &user).await?;
    let config =
        create_node_config(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    // Create node in lab1
    let mut node = create_node(
        &db,
        "node1",
        1,
        config.id.unwrap(),
        lab1.id.clone().unwrap(),
    )
    .await?;

    // Try to change lab
    node.lab = lab2.id.unwrap();

    let result = update_node(&db, node).await;

    assert!(
        result.is_err(),
        "Should fail when trying to change lab (immutable field)"
    );
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("lab") && error_msg.contains("immutable"),
        "Error should mention lab field is immutable, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_node_duplicate_name_fails() -> Result<()> {
    let db = setup_db("test_update_node_dup_name").await?;

    let user = create_user(&db, "diana".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Lab", "lab-0005", &user).await?;
    let config =
        create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create two nodes
    create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    let mut node2 = create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Try to update node2 with node1's name
    node2.name = "node1".to_string();

    let result = update_node(&db, node2).await;

    assert!(
        result.is_err(),
        "Should fail on duplicate (name, lab) combination"
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_node_duplicate_index_fails() -> Result<()> {
    let db = setup_db("test_update_node_dup_index").await?;

    let user = create_user(&db, "emily".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Lab", "lab-0006", &user).await?;
    let config =
        create_node_config(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    // Create two nodes
    create_node(
        &db,
        "router1",
        10,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    let mut router2 = create_node(
        &db,
        "router2",
        20,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Try to update router2 with router1's index
    router2.index = 10;

    let result = update_node(&db, router2).await;

    assert!(
        result.is_err(),
        "Should fail on duplicate (index, lab) combination"
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_node_preserves_id() -> Result<()> {
    let db = setup_db("test_update_node_id").await?;

    let user = create_user(&db, "frank".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Lab", "lab-0007", &user).await?;
    let config =
        create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create node
    let mut node = create_node(
        &db,
        "node1",
        1,
        config.id.unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let original_id = node.id.clone();

    // Update node
    node.name = "updated".to_string();
    let updated = update_node(&db, node).await?;

    assert_eq!(updated.id, original_id, "ID should not change");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_node_can_change_config() -> Result<()> {
    let db = setup_db("test_update_node_config").await?;

    let user = create_user(&db, "grace".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Lab", "lab-0008", &user).await?;
    let config1 =
        create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;
    let config2 =
        create_node_config(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    // Create node with config1
    let mut node = create_node(
        &db,
        "node1",
        1,
        config1.id.unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Update to config2
    let config2_id = config2.id.clone().unwrap();
    node.config = config2_id.clone();
    let updated = update_node(&db, node).await?;

    assert_eq!(updated.config, config2_id);

    // Verify by retrieving
    let retrieved = get_node(&db, updated.id.unwrap()).await?;
    assert_eq!(retrieved.config, config2_id);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_node_nonexistent_fails() -> Result<()> {
    let db = setup_db("test_update_node_nonexistent").await?;

    let user = create_user(&db, "hannah".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Lab", "lab-0009", &user).await?;
    let config =
        create_node_config(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create node
    let mut node = create_node(
        &db,
        "node1",
        1,
        config.id.unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Change ID to nonexistent
    node.id = Some(("node", "nonexistent").into());

    let result = update_node(&db, node).await;

    assert!(result.is_err(), "Should fail when node doesn't exist");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found") || error_msg.contains("Node not found"),
        "Error should mention node not found, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}
