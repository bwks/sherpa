use anyhow::Result;
use db::{
    count_nodes, count_nodes_by_lab, create_lab, create_node, create_node_image, create_user,
    get_node, get_node_by_name_and_lab,
};
use shared::data::{NodeConfig, NodeModel};

use crate::helper::{setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_create_node_success() -> Result<()> {
    let db = setup_db("test_create_node").await?;

    // Setup dependencies
    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user, "127.127.1.0/24").await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create node
    let node = create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    assert_eq!(node.name, "node1");
    assert_eq!(node.index, 1);
    assert_eq!(node.image, config.id.unwrap());
    assert_eq!(node.lab, lab.id.unwrap());
    assert!(node.id.is_some(), "Node should have an ID");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_node_with_lab() -> Result<()> {
    let db = setup_db("test_create_node_lab").await?;

    let user = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Bob's Lab", "lab-0002", &user, "127.127.1.0/24").await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    let node = create_node(
        &db,
        "router1",
        0,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Verify node is associated with lab
    assert_eq!(node.lab, lab.id.unwrap());
    assert_eq!(node.image, config.id.unwrap());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_node_duplicate_name_per_lab_fails() -> Result<()> {
    let db = setup_db("test_create_node_dup_name").await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Lab One", "lab-0003", &user, "127.127.1.0/24").await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create first node
    create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Try to create node with same name in same lab
    let result = create_node(
        &db,
        "node1",
        2,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await;

    assert!(
        result.is_err(),
        "Should fail on duplicate (name, lab) combination"
    );
    let error_msg = result.unwrap_err().to_string();
    println!("Duplicate name+lab error: {}", error_msg);
    assert!(
        error_msg.contains("Failed to create node") && error_msg.contains("node1"),
        "Error should mention the failed node creation, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_node_duplicate_index_per_lab_fails() -> Result<()> {
    let db = setup_db("test_create_node_dup_index").await?;

    let user = create_user(&db, "diana".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Lab Two", "lab-0004", &user, "127.127.1.0/24").await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    // Create first node
    create_node(
        &db,
        "router1",
        10,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Try to create node with same index in same lab
    let result = create_node(
        &db,
        "router2",
        10,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await;

    assert!(
        result.is_err(),
        "Should fail on duplicate (index, lab) combination"
    );
    let error_msg = result.unwrap_err().to_string();
    println!("Duplicate index+lab error: {}", error_msg);
    assert!(
        error_msg.contains("Failed to create node") && error_msg.contains("router2"),
        "Error should mention the failed node creation, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_node_same_name_different_labs_succeeds() -> Result<()> {
    let db = setup_db("test_create_node_diff_labs").await?;

    let user = create_user(&db, "emily".to_string(), "TestPass123!", false, vec![]).await?;
    let lab1 = create_lab(&db, "Lab 1", "lab-0005", &user, "127.127.1.0/24").await?;
    let lab2 = create_lab(&db, "Lab 2", "lab-0006", &user, "127.127.2.0/24").await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create node in lab1
    let node1 = create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab1.id.clone().unwrap(),
    )
    .await?;

    // Create node with same name in lab2 - should succeed
    let node2 = create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab2.id.clone().unwrap(),
    )
    .await?;

    assert_eq!(node1.name, "node1");
    assert_eq!(node2.name, "node1");
    assert_ne!(node1.lab, node2.lab, "Nodes should be in different labs");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_node_with_index_boundaries() -> Result<()> {
    let db = setup_db("test_create_node_index").await?;

    let user = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Lab Test", "lab-0007", &user, "127.127.1.0/24").await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    // Test minimum index (0)
    let node1 = create_node(
        &db,
        "node_min",
        0,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    assert_eq!(node1.index, 0);

    // Test maximum index (65535)
    let node2 = create_node(
        &db,
        "node_max",
        65535,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    assert_eq!(node2.index, 65535);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_node_increments_count() -> Result<()> {
    let db = setup_db("test_create_node_count").await?;

    let user = create_user(&db, "grace".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Lab Count", "lab-0008", &user, "127.127.1.0/24").await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    let initial_count = count_nodes(&db).await?;
    let initial_lab_count = count_nodes_by_lab(&db, lab.id.clone().unwrap()).await?;

    // Create nodes
    create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;
    create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let final_count = count_nodes(&db).await?;
    let final_lab_count = count_nodes_by_lab(&db, lab.id.clone().unwrap()).await?;

    assert_eq!(final_count, initial_count + 2);
    assert_eq!(final_lab_count, initial_lab_count + 2);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_node_can_retrieve_by_name_and_lab() -> Result<()> {
    let db = setup_db("test_create_node_retrieve").await?;

    let user = create_user(&db, "hannah".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Lab Retrieve", "lab-0009", &user, "127.127.1.0/24").await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    // Create node
    let created_node = create_node(
        &db,
        "switch1",
        5,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Retrieve by name and lab
    let retrieved_node = get_node_by_name_and_lab(&db, "switch1", lab.id.clone().unwrap()).await?;

    assert_eq!(created_node.id, retrieved_node.id);
    assert_eq!(created_node.name, retrieved_node.name);
    assert_eq!(created_node.index, retrieved_node.index);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_node_can_retrieve_by_id() -> Result<()> {
    let db = setup_db("test_create_node_retrieve_id").await?;

    let user = create_user(&db, "ian".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Lab Retrieve ID", "lab-0010", &user, "127.127.1.0/24").await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create node
    let created_node = create_node(
        &db,
        "server1",
        7,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    // Retrieve by ID
    let node_id = created_node.id.clone().unwrap();
    let retrieved_node = get_node(&db, node_id).await?;

    assert_eq!(created_node.id, retrieved_node.id);
    assert_eq!(created_node.name, retrieved_node.name);
    assert_eq!(created_node.index, retrieved_node.index);

    teardown_db(&db).await?;
    Ok(())
}
