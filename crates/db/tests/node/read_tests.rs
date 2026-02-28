use anyhow::Result;
use db::{
    count_nodes, count_nodes_by_lab, create_lab, create_node, create_node_image, create_user,
    get_node, get_node_by_id, get_node_by_name_and_lab, list_nodes, list_nodes_by_lab,
};
use shared::data::{NodeConfig, NodeModel, RecordId};

use crate::helper::{setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_get_node_by_id_success() -> Result<()> {
    let db = setup_db("test_get_node_id").await?;

    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    let created = create_node(&db, "node1", 1, config.id.unwrap(), lab.id.clone().unwrap()).await?;

    let node_id = created.id.clone().unwrap();
    let retrieved = get_node_by_id(&db, node_id).await?;

    assert_eq!(created.id, retrieved.id);
    assert_eq!(created.name, retrieved.name);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_node_not_found() -> Result<()> {
    let db = setup_db("test_get_node_not_found").await?;

    let fake_id = RecordId::new("node", "nonexistent");
    let result = get_node(&db, fake_id).await;

    assert!(result.is_err(), "Should fail when node not found");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found"),
        "Error should mention node not found, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_node_by_name_and_lab_success() -> Result<()> {
    let db = setup_db("test_get_node_name_lab").await?;

    let user = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Bob's Lab", "lab-0002", &user).await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    let created = create_node(
        &db,
        "router1",
        5,
        config.id.unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let retrieved = get_node_by_name_and_lab(&db, "router1", lab.id.unwrap()).await?;

    assert_eq!(created.id, retrieved.id);
    assert_eq!(created.name, retrieved.name);
    assert_eq!(created.index, retrieved.index);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_node_by_name_and_lab_not_found() -> Result<()> {
    let db = setup_db("test_get_node_name_not_found").await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Lab", "lab-0003", &user).await?;

    let result = get_node_by_name_and_lab(&db, "nonexistent", lab.id.unwrap()).await;

    assert!(result.is_err(), "Should fail when node not found");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found") && error_msg.contains("nonexistent"),
        "Error should mention node not found, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_nodes_empty() -> Result<()> {
    let db = setup_db("test_list_nodes_empty").await?;

    let nodes = list_nodes(&db).await?;

    assert_eq!(nodes.len(), 0, "Should have no nodes initially");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_nodes_returns_all() -> Result<()> {
    let db = setup_db("test_list_nodes_all").await?;

    let user = create_user(&db, "diana".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Lab", "lab-0004", &user).await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create multiple nodes
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
    create_node(
        &db,
        "node3",
        3,
        config.id.clone().unwrap(),
        lab.id.clone().unwrap(),
    )
    .await?;

    let nodes = list_nodes(&db).await?;

    assert_eq!(nodes.len(), 3, "Should have 3 nodes");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_nodes_by_lab_empty() -> Result<()> {
    let db = setup_db("test_list_nodes_lab_empty").await?;

    let user = create_user(&db, "emily".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Empty Lab", "lab-0005", &user).await?;

    let nodes = list_nodes_by_lab(&db, lab.id.unwrap()).await?;

    assert_eq!(nodes.len(), 0, "Should have no nodes in empty lab");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_nodes_by_lab_filters_correctly() -> Result<()> {
    let db = setup_db("test_list_nodes_lab_filter").await?;

    let user = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;
    let lab1 = create_lab(&db, "Lab 1", "lab-0006", &user).await?;
    let lab2 = create_lab(&db, "Lab 2", "lab-0007", &user).await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    // Create nodes in lab1
    create_node(
        &db,
        "node1",
        1,
        config.id.clone().unwrap(),
        lab1.id.clone().unwrap(),
    )
    .await?;
    create_node(
        &db,
        "node2",
        2,
        config.id.clone().unwrap(),
        lab1.id.clone().unwrap(),
    )
    .await?;

    // Create nodes in lab2
    create_node(
        &db,
        "node3",
        1,
        config.id.clone().unwrap(),
        lab2.id.clone().unwrap(),
    )
    .await?;

    let lab1_nodes = list_nodes_by_lab(&db, lab1.id.unwrap()).await?;
    let lab2_nodes = list_nodes_by_lab(&db, lab2.id.unwrap()).await?;

    assert_eq!(lab1_nodes.len(), 2, "Lab1 should have 2 nodes");
    assert_eq!(lab2_nodes.len(), 1, "Lab2 should have 1 node");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_nodes_zero() -> Result<()> {
    let db = setup_db("test_count_nodes_zero").await?;

    let count = count_nodes(&db).await?;

    assert_eq!(count, 0, "Should have no nodes initially");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_nodes_accurate() -> Result<()> {
    let db = setup_db("test_count_nodes").await?;

    let user = create_user(&db, "grace".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Lab", "lab-0008", &user).await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;

    // Create nodes
    for i in 1..=5 {
        create_node(
            &db,
            &format!("node{}", i),
            i as u16,
            config.id.clone().unwrap(),
            lab.id.clone().unwrap(),
        )
        .await?;
    }

    let count = count_nodes(&db).await?;

    assert_eq!(count, 5, "Should have 5 nodes");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_nodes_by_lab_zero() -> Result<()> {
    let db = setup_db("test_count_nodes_lab_zero").await?;

    let user = create_user(&db, "hannah".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Empty Lab", "lab-0009", &user).await?;

    let count = count_nodes_by_lab(&db, lab.id.unwrap()).await?;

    assert_eq!(count, 0, "Should have no nodes in empty lab");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_nodes_by_lab_accurate() -> Result<()> {
    let db = setup_db("test_count_nodes_lab").await?;

    let user = create_user(&db, "ian".to_string(), "TestPass123!", false, vec![]).await?;
    let lab1 = create_lab(&db, "Lab 1", "lab-0010", &user).await?;
    let lab2 = create_lab(&db, "Lab 2", "lab-0011", &user).await?;
    let config = create_node_image(&db, NodeConfig::get_model(NodeModel::CiscoIosv)).await?;

    // Create nodes in lab1
    for i in 1..=3 {
        create_node(
            &db,
            &format!("node{}", i),
            i as u16,
            config.id.clone().unwrap(),
            lab1.id.clone().unwrap(),
        )
        .await?;
    }

    // Create nodes in lab2
    for i in 1..=2 {
        create_node(
            &db,
            &format!("node{}", i),
            i as u16,
            config.id.clone().unwrap(),
            lab2.id.clone().unwrap(),
        )
        .await?;
    }

    let lab1_count = count_nodes_by_lab(&db, lab1.id.unwrap()).await?;
    let lab2_count = count_nodes_by_lab(&db, lab2.id.unwrap()).await?;
    let total_count = count_nodes(&db).await?;

    assert_eq!(lab1_count, 3, "Lab1 should have 3 nodes");
    assert_eq!(lab2_count, 2, "Lab2 should have 2 nodes");
    assert_eq!(total_count, 5, "Total should be 5 nodes");

    teardown_db(&db).await?;
    Ok(())
}
