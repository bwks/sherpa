use anyhow::Result;
use db::{
    DbError, DefaultRequest, LabDetails, count_nodes_by_lab, create_lab, create_node, create_user,
    get_node, get_node_by_name_and_lab, upsert_node_image,
};
use shared::data::{ModelKind, NodeModel, RecordId};

use crate::helper::{ipv4_lab_networks, setup_db};

#[tokio::test]
async fn test_get_node_not_found() -> Result<()> {
    let db = setup_db().await?;

    let fake_id = RecordId::new("node", "nonexistent");
    let error = get_node(&db, fake_id.clone()).await.unwrap_err();

    assert_eq!(
        error.downcast_ref::<DbError>(),
        Some(&DbError::NodeNotFound { id: fake_id }),
        "{error:#}"
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
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;

    let lab_id = lab.id.unwrap();
    let error = get_node_by_name_and_lab(&db, "nonexistent", lab_id.clone())
        .await
        .unwrap_err();

    assert_eq!(
        error.downcast_ref::<DbError>(),
        Some(&DbError::NodeNotFoundInLab {
            name: "nonexistent".to_owned(),
            lab: lab_id,
        }),
        "{error:#}"
    );

    Ok(())
}

#[tokio::test]
async fn labs_list_012_the_node_count_is_the_labs_own_nodes() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "ian".to_string(), "TestPass123!", false, vec![]).await?;
    let lab1 = create_lab(
        &db,
        "Lab 1",
        "lab-0010",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let lab2 = create_lab(
        &db,
        "Lab 2",
        "lab-0011",
        &user,
        &ipv4_lab_networks("172.31.2.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let config = upsert_node_image(
        &db,
        ModelKind::default_for(NodeModel::CiscoIos).node_config(),
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
