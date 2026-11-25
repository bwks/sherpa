use anyhow::Result;
use db::{
    DbError, DefaultRequest, LabDetails, create_lab, create_node, create_user, get_node,
    upsert_node_image,
};
use shared::data::{ModelKind, NodeModel};

use crate::helper::{ipv4_lab_networks, setup_db};

#[tokio::test]
async fn test_create_node_duplicate_name_per_lab_fails() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab One",
        "lab-0003",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let config = upsert_node_image(
        &db,
        ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
        DefaultRequest::Keep,
    )
    .await?;

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
    let error = result.unwrap_err();
    assert_eq!(
        error.downcast_ref::<DbError>(),
        Some(&DbError::NodeNotCreated {
            name: "node1".to_owned(),
            index: 2,
            lab: lab.id.clone().unwrap(),
        }),
        "{error:#}"
    );

    Ok(())
}

#[tokio::test]
async fn test_create_node_duplicate_index_per_lab_fails() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "diana".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab Two",
        "lab-0004",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let config = upsert_node_image(
        &db,
        ModelKind::default_for(NodeModel::CiscoIos).node_config(),
        DefaultRequest::Keep,
    )
    .await?;

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
    let error = result.unwrap_err();
    assert_eq!(
        error.downcast_ref::<DbError>(),
        Some(&DbError::NodeNotCreated {
            name: "router2".to_owned(),
            index: 10,
            lab: lab.id.clone().unwrap(),
        }),
        "{error:#}"
    );

    Ok(())
}

#[tokio::test]
async fn test_create_node_same_name_different_labs_succeeds() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "emily".to_string(), "TestPass123!", false, vec![]).await?;
    let lab1 = create_lab(
        &db,
        "Lab 1",
        "lab-0005",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let lab2 = create_lab(
        &db,
        "Lab 2",
        "lab-0006",
        &user,
        &ipv4_lab_networks("172.31.2.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let config = upsert_node_image(
        &db,
        ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
        DefaultRequest::Keep,
    )
    .await?;

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

    Ok(())
}

#[tokio::test]
async fn test_create_node_with_index_boundaries() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab Test",
        "lab-0007",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let config = upsert_node_image(
        &db,
        ModelKind::default_for(NodeModel::CiscoIos).node_config(),
        DefaultRequest::Keep,
    )
    .await?;

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

    Ok(())
}

#[tokio::test]
async fn test_create_node_can_retrieve_by_id() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "ian".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab Retrieve ID",
        "lab-0010",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let config = upsert_node_image(
        &db,
        ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
        DefaultRequest::Keep,
    )
    .await?;

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

    Ok(())
}
