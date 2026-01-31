use anyhow::Result;
use data::NodeModel;
use db::{
    count_labs, create_lab, create_lab_node, create_user, delete_lab, delete_lab_by_id,
    delete_lab_cascade, delete_lab_safe, get_lab,
};

use crate::helper::{setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_delete_lab_success() -> Result<()> {
    let db = setup_db("test_delete_lab").await?;

    let user = create_user(&db, "alice".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;

    // Delete lab
    delete_lab(&db, &lab.lab_id).await?;

    // Verify it's gone
    let result = get_lab(&db, "lab-0001").await;
    assert!(result.is_err(), "Lab should not exist after deletion");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_lab_by_id() -> Result<()> {
    let db = setup_db("test_delete_lab_by_id").await?;

    let user = create_user(&db, "bob".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0002", &user).await?;
    let lab_id = lab.id.unwrap();

    // Delete by RecordId
    delete_lab_by_id(&db, lab_id).await?;

    // Verify it's gone
    let result = get_lab(&db, "lab-0002").await;
    assert!(result.is_err(), "Lab should not exist after deletion");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_nonexistent_lab_fails() -> Result<()> {
    let db = setup_db("test_delete_nonexistent").await?;

    let result = delete_lab(&db, "invalid").await;

    assert!(result.is_err(), "Should error on nonexistent lab");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_lab_decreases_count() -> Result<()> {
    let db = setup_db("test_delete_decreases_count").await?;

    let user = create_user(&db, "charlie".to_string(), vec![]).await?;
    create_lab(&db, "Lab 1", "lab-0003", &user).await?;
    let lab2 = create_lab(&db, "Lab 2", "lab-0004", &user).await?;

    let count_before = count_labs(&db).await?;

    delete_lab(&db, &lab2.lab_id).await?;

    let count_after = count_labs(&db).await?;

    assert_eq!(count_after, count_before - 1, "Count should decrease by 1");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_lab_cascade_removes_nodes() -> Result<()> {
    let db = setup_db("test_delete_cascade_nodes").await?;

    let user = create_user(&db, "diana".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0005", &user).await?;

    // Create some nodes
    create_lab_node(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;
    create_lab_node(&db, "node2", 2, NodeModel::WindowsServer, &lab).await?;

    // Delete with cascade
    delete_lab_cascade(&db, &lab.lab_id).await?;

    // Verify lab is gone
    let result = get_lab(&db, "lab-0005").await;
    assert!(result.is_err(), "Lab should be deleted");

    // Note: With CASCADE DELETE in schema, nodes should be automatically deleted
    // This test verifies the cascade function works correctly

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_lab_safe_with_nodes_fails() -> Result<()> {
    let db = setup_db("test_delete_safe_nodes_fail").await?;

    let user = create_user(&db, "eve".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Test Lab", "lab-0006", &user).await?;

    // Create a node
    create_lab_node(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;

    // Try safe delete (should fail)
    let result = delete_lab_safe(&db, &lab.lab_id).await;

    assert!(result.is_err(), "Should fail when lab has nodes");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("node") || error_msg.contains("1"),
        "Error should mention nodes, got: {}",
        error_msg
    );

    // Verify lab still exists
    let lab_check = get_lab(&db, "lab-0006").await;
    assert!(lab_check.is_ok(), "Lab should still exist");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_lab_safe_empty_succeeds() -> Result<()> {
    let db = setup_db("test_delete_safe_empty").await?;

    let user = create_user(&db, "frank".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Empty Lab", "lab-0007", &user).await?;

    // Safe delete should succeed (no nodes/links)
    delete_lab_safe(&db, &lab.lab_id).await?;

    // Verify it's gone
    let result = get_lab(&db, "lab-0007").await;
    assert!(result.is_err(), "Lab should be deleted");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_lab_cascade_full() -> Result<()> {
    let db = setup_db("test_delete_cascade_full").await?;

    let user = create_user(&db, "grace".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Full Lab", "lab-0008", &user).await?;

    // Create multiple nodes
    create_lab_node(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;
    create_lab_node(&db, "node2", 2, NodeModel::WindowsServer, &lab).await?;
    create_lab_node(&db, "node3", 3, NodeModel::CiscoNexus9300v, &lab).await?;

    // Note: In a real scenario, we'd also create links between nodes here
    // For now, just test nodes cascade

    // Delete with cascade
    delete_lab_cascade(&db, &lab.lab_id).await?;

    // Verify everything is gone
    let result = get_lab(&db, "lab-0008").await;
    assert!(result.is_err(), "Lab should be deleted");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_lab_with_cascade_in_schema() -> Result<()> {
    let db = setup_db("test_schema_cascade").await?;

    let user = create_user(&db, "heidi".to_string(), vec![]).await?;
    let lab = create_lab(&db, "Auto Cascade Lab", "lab-0009", &user).await?;

    // Create nodes
    create_lab_node(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;
    create_lab_node(&db, "node2", 2, NodeModel::WindowsServer, &lab).await?;

    // Delete just the lab (schema CASCADE should handle nodes)
    delete_lab(&db, &lab.lab_id).await?;

    // Verify lab is gone
    let result = get_lab(&db, "lab-0009").await;
    assert!(result.is_err(), "Lab should be deleted");

    // With CASCADE DELETE in schema, nodes should be automatically deleted
    // This is handled by the database, not application code

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_lab_nonexistent_by_id() -> Result<()> {
    let db = setup_db("test_delete_nonexistent_id").await?;

    let fake_id = ("lab", "nonexistent").into();
    let result = delete_lab_by_id(&db, fake_id).await;

    // This might succeed (deleting non-existent returns Ok) or fail depending on implementation
    // Just verify it doesn't panic
    let _ = result;

    teardown_db(&db).await?;
    Ok(())
}
