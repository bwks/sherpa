use anyhow::Result;
use db::{
    DefaultRequest, LabDetails, count_nodes_by_lab, create_lab, create_node, create_user, get_node,
    get_node_by_name_and_lab, list_nodes_by_lab, upsert_node_image,
};
use shared::data::{NodeConfig, NodeModel, RecordId};

use crate::helper::{ipv4_lab_networks, setup_db};

#[tokio::test]
async fn test_get_node_not_found() -> Result<()> {
    let db = setup_db().await?;

    let fake_id = RecordId::new("node", "nonexistent");
    let result = get_node(&db, fake_id).await;

    assert!(result.is_err(), "Should fail when node not found");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found"),
        "Error should mention node not found, got: {error_msg}"
    );

    Ok(())
}

#[tokio::test]
async fn test_get_node_by_name_and_lab_success() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Bob's Lab",
        "lab-0002",
        &user,
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabDetails::default(),
    )
    .await?;
    let config = upsert_node_image(
        &db,
        NodeConfig::get_model(NodeModel::CiscoIosv),
        DefaultRequest::Keep,
    )
    .await?;

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

    Ok(())
}

#[tokio::test]
async fn test_get_node_by_name_and_lab_not_found() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Lab",
        "lab-0003",
        &user,
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabDetails::default(),
    )
    .await?;

    let result = get_node_by_name_and_lab(&db, "nonexistent", lab.id.unwrap()).await;

    assert!(result.is_err(), "Should fail when node not found");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found") && error_msg.contains("nonexistent"),
        "Error should mention node not found, got: {error_msg}"
    );

    Ok(())
}

#[tokio::test]
async fn test_list_nodes_by_lab_empty() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "emily".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Empty Lab",
        "lab-0005",
        &user,
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabDetails::default(),
    )
    .await?;

    let nodes = list_nodes_by_lab(&db, lab.id.unwrap()).await?;

    assert_eq!(nodes.len(), 0, "Should have no nodes in empty lab");

    Ok(())
}

#[tokio::test]
async fn test_list_nodes_by_lab_filters_correctly() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;
    let lab1 = create_lab(
        &db,
        "Lab 1",
        "lab-0006",
        &user,
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabDetails::default(),
    )
    .await?;
    let lab2 = create_lab(
        &db,
        "Lab 2",
        "lab-0007",
        &user,
        &ipv4_lab_networks(
            "127.127.2.0/24",
            "172.31.2.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabDetails::default(),
    )
    .await?;
    let config = upsert_node_image(
        &db,
        NodeConfig::get_model(NodeModel::CiscoIosv),
        DefaultRequest::Keep,
    )
    .await?;

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

    Ok(())
}

#[tokio::test]
async fn test_count_nodes_by_lab_zero() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "hannah".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Empty Lab",
        "lab-0009",
        &user,
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabDetails::default(),
    )
    .await?;

    let count = count_nodes_by_lab(&db, lab.id.unwrap()).await?;

    assert_eq!(count, 0, "Should have no nodes in empty lab");

    Ok(())
}

#[tokio::test]
async fn test_count_nodes_by_lab_accurate() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "ian".to_string(), "TestPass123!", false, vec![]).await?;
    let lab1 = create_lab(
        &db,
        "Lab 1",
        "lab-0010",
        &user,
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabDetails::default(),
    )
    .await?;
    let lab2 = create_lab(
        &db,
        "Lab 2",
        "lab-0011",
        &user,
        &ipv4_lab_networks(
            "127.127.2.0/24",
            "172.31.2.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabDetails::default(),
    )
    .await?;
    let config = upsert_node_image(
        &db,
        NodeConfig::get_model(NodeModel::CiscoIosv),
        DefaultRequest::Keep,
    )
    .await?;

    // Create nodes in lab1
    for i in 1_u16..=3 {
        create_node(
            &db,
            &format!("node{i}"),
            i,
            config.id.clone().unwrap(),
            lab1.id.clone().unwrap(),
        )
        .await?;
    }

    // Create nodes in lab2
    for i in 1_u16..=2 {
        create_node(
            &db,
            &format!("node{i}"),
            i,
            config.id.clone().unwrap(),
            lab2.id.clone().unwrap(),
        )
        .await?;
    }

    let lab1_count = count_nodes_by_lab(&db, lab1.id.unwrap()).await?;
    let lab2_count = count_nodes_by_lab(&db, lab2.id.unwrap()).await?;

    assert_eq!(lab1_count, 3, "Lab1 should have 3 nodes");
    assert_eq!(lab2_count, 2, "Lab2 should have 2 nodes");

    Ok(())
}
