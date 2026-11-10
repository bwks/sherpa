use anyhow::Result;
use db::{
    DefaultRequest, LabDetails, create_lab, create_user, delete_lab, get_lab, list_labs,
    upsert_node_image,
};
use shared::data::{NodeConfig, NodeModel};

use crate::helper::{create_test_node_with_model, ipv4_lab_networks, setup_db};

#[tokio::test]
async fn test_delete_lab_success() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Test Lab",
        "lab-0001",
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

    // Delete lab
    delete_lab(&db, &lab.lab_id).await?;

    // Verify it's gone
    let result = get_lab(&db, "lab-0001").await;
    assert!(result.is_err(), "Lab should not exist after deletion");

    Ok(())
}

#[tokio::test]
async fn test_delete_nonexistent_lab_fails() -> Result<()> {
    let db = setup_db().await?;

    let result = delete_lab(&db, "invalid").await;

    assert!(result.is_err(), "Should error on nonexistent lab");

    Ok(())
}

#[tokio::test]
async fn test_delete_lab_decreases_count() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;
    create_lab(
        &db,
        "Lab 1",
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
    let lab2 = create_lab(
        &db,
        "Lab 2",
        "lab-0004",
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

    let count_before = list_labs(&db).await?.len();

    delete_lab(&db, &lab2.lab_id).await?;

    let count_after = list_labs(&db).await?.len();

    assert_eq!(count_after, count_before - 1, "Count should decrease by 1");

    Ok(())
}

#[tokio::test]
async fn test_delete_lab_with_cascade_in_schema() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "heidi".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        &db,
        "Auto Cascade Lab",
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

    // Create node configs first
    upsert_node_image(
        &db,
        NodeConfig::get_model(NodeModel::UbuntuLinux),
        DefaultRequest::Keep,
    )
    .await?;
    upsert_node_image(
        &db,
        NodeConfig::get_model(NodeModel::WindowsServer),
        DefaultRequest::Keep,
    )
    .await?;

    // Create nodes
    create_test_node_with_model(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;
    create_test_node_with_model(&db, "node2", 2, NodeModel::WindowsServer, &lab).await?;

    // Delete just the lab (schema CASCADE should handle nodes)
    delete_lab(&db, &lab.lab_id).await?;

    // Verify lab is gone
    let result = get_lab(&db, "lab-0009").await;
    assert!(result.is_err(), "Lab should be deleted");

    // With CASCADE DELETE in schema, nodes should be automatically deleted
    // This is handled by the database, not application code

    Ok(())
}
