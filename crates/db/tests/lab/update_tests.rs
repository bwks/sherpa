use anyhow::Result;
use db::{create_lab, create_user, get_lab, update_lab};
use shared::data::{DbLab, RecordId};

use crate::helper::{setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_update_lab_success() -> Result<()> {
    let db = setup_db("test_update_lab").await?;

    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let mut lab = create_lab(
        &db,
        "Original Name",
        "lab-0001",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;

    // Update the lab name
    lab.name = "Updated Name".to_string();
    let updated = update_lab(&db, lab).await?;

    assert_eq!(updated.name, "Updated Name");
    assert_eq!(updated.lab_id, "lab-0001");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_lab_without_id_fails() -> Result<()> {
    let db = setup_db("test_update_no_id").await?;

    let user = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    let user_id = user.id.unwrap();

    let lab = DbLab {
        id: None,
        lab_id: "lab-0002".to_string(),
        name: "Test Lab".to_string(),
        user: user_id,
        loopback_network: "127.127.1.0/24".to_string(),
        management_network: "172.31.1.0/24".to_string(),
    };

    let result = update_lab(&db, lab).await;

    assert!(result.is_err(), "Should fail without ID");
    assert!(result.unwrap_err().to_string().contains("without id field"));

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_nonexistent_lab_fails() -> Result<()> {
    let db = setup_db("test_update_nonexistent").await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;
    let fake_id = RecordId::new("lab", "nonexistent");

    let lab = DbLab {
        id: Some(fake_id),
        lab_id: "lab-0003".to_string(),
        name: "Test Lab".to_string(),
        user: user.id.unwrap(),
        loopback_network: "127.127.1.0/24".to_string(),
        management_network: "172.31.1.0/24".to_string(),
    };

    let result = update_lab(&db, lab).await;

    assert!(result.is_err(), "Should fail on nonexistent lab");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_preserves_id() -> Result<()> {
    let db = setup_db("test_update_preserves_id").await?;

    let user = create_user(&db, "diana".to_string(), "TestPass123!", false, vec![]).await?;
    let mut lab = create_lab(
        &db,
        "Original",
        "lab-0004",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let original_id = lab.id.clone();

    lab.name = "Updated".to_string();
    let updated = update_lab(&db, lab).await?;

    assert_eq!(updated.id, original_id, "ID should not change");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_lab_id() -> Result<()> {
    let db = setup_db("test_update_lab_id").await?;

    let user = create_user(&db, "eve".to_string(), "TestPass123!", false, vec![]).await?;
    let mut lab = create_lab(
        &db,
        "Test Lab",
        "lab-0005",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;

    // Update lab_id (business key)
    lab.lab_id = "lab-0099".to_string();
    let updated = update_lab(&db, lab).await?;

    assert_eq!(updated.lab_id, "lab-0099");

    // Verify we can get it by new lab_id
    let fetched = get_lab(&db, "lab-0099").await?;
    assert_eq!(fetched.id, updated.id);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_cannot_change_owner() -> Result<()> {
    let db = setup_db("test_update_owner_immutable").await?;

    let user1 = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;
    let user2 = create_user(&db, "grace".to_string(), "TestPass123!", false, vec![]).await?;

    let mut lab = create_lab(
        &db,
        "Frank's Lab",
        "lab-0006",
        &user1,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;

    // Try to change owner
    lab.user = user2.id.unwrap();
    let result = update_lab(&db, lab).await;

    assert!(result.is_err(), "Should fail when trying to change owner");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("immutable") || error_msg.contains("owner"),
        "Error should mention owner immutability, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_lab_name_constraint() -> Result<()> {
    let db = setup_db("test_update_name_constraint").await?;

    let user = create_user(&db, "heidi".to_string(), "TestPass123!", false, vec![]).await?;

    // Create two labs
    let mut lab1 = create_lab(
        &db,
        "Lab 1",
        "lab-0007",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    create_lab(
        &db,
        "Lab 2",
        "lab-0008",
        &user,
        "127.127.2.0/24",
        "172.31.2.0/24",
    )
    .await?;

    // Try to update lab1 to have the same name as lab2
    lab1.name = "Lab 2".to_string();
    let result = update_lab(&db, lab1).await;

    assert!(
        result.is_err(),
        "Should fail on duplicate (name, user) constraint"
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_lab_invalid_lab_id() -> Result<()> {
    let db = setup_db("test_update_invalid_lab_id").await?;

    let user = create_user(&db, "ivan".to_string(), "TestPass123!", false, vec![]).await?;
    let mut lab = create_lab(
        &db,
        "Test Lab",
        "lab-0009",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;

    // Try to update to invalid lab_id
    lab.lab_id = "short".to_string();
    let result = update_lab(&db, lab).await;

    assert!(result.is_err(), "Should fail on invalid lab_id validation");
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("exactly 8 characters")
    );

    teardown_db(&db).await?;
    Ok(())
}
