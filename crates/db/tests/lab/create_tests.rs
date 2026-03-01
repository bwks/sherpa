use anyhow::Result;
use db::{count_labs, create_lab, create_user, get_lab, upsert_lab, validate_lab_id};
use shared::data::DbLab;

use crate::helper::{setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_create_lab_success() -> Result<()> {
    let db = setup_db("test_create_lab").await?;

    // Create a user first
    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;

    // Create lab
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user, "127.127.1.0/24").await?;

    assert_eq!(lab.name, "Test Lab");
    assert_eq!(lab.lab_id, "lab-0001");
    assert_eq!(lab.user, user.id.unwrap());
    assert!(lab.id.is_some(), "Lab should have an ID");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_lab_with_user() -> Result<()> {
    let db = setup_db("test_create_lab_user").await?;

    let user = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(&db, "Bob's Lab", "lab-0002", &user, "127.127.1.0/24").await?;

    // Verify lab is associated with user
    assert_eq!(lab.user, user.id.unwrap());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_lab_duplicate_lab_id_fails() -> Result<()> {
    let db = setup_db("test_create_lab_dup_id").await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;

    // Create first lab
    create_lab(&db, "Lab One", "lab-0003", &user, "127.127.1.0/24").await?;

    // Try to create lab with same lab_id
    let result = create_lab(&db, "Lab Two", "lab-0003", &user, "127.127.2.0/24").await;

    assert!(result.is_err(), "Should fail on duplicate lab_id");
    let error_msg = result.unwrap_err().to_string();
    println!("Duplicate lab_id error: {}", error_msg);
    assert!(
        error_msg.contains("Failed to create lab") && error_msg.contains("Lab Two"),
        "Error should mention the failed lab creation, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_lab_duplicate_name_per_user_fails() -> Result<()> {
    let db = setup_db("test_create_lab_dup_name").await?;

    let user = create_user(&db, "diana".to_string(), "TestPass123!", false, vec![]).await?;

    // Create first lab
    create_lab(&db, "My Lab", "lab-0004", &user, "127.127.1.0/24").await?;

    // Try to create lab with same name for same user
    let result = create_lab(&db, "My Lab", "lab-0005", &user, "127.127.2.0/24").await;

    assert!(
        result.is_err(),
        "Should fail on duplicate (name, user) combination"
    );
    let error_msg = result.unwrap_err().to_string();
    println!("Duplicate name+user error: {}", error_msg);
    assert!(
        error_msg.contains("Failed to create lab") && error_msg.contains("My Lab"),
        "Error should mention the failed lab creation, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_lab_same_name_different_users_succeeds() -> Result<()> {
    let db = setup_db("test_create_lab_diff_users").await?;

    let user1 = create_user(&db, "eve".to_string(), "TestPass123!", false, vec![]).await?;
    let user2 = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;

    // Both users can have labs with the same name
    let lab1 = create_lab(&db, "Shared Name", "lab-0006", &user1, "127.127.1.0/24").await?;
    let lab2 = create_lab(&db, "Shared Name", "lab-0007", &user2, "127.127.2.0/24").await?;

    assert_eq!(lab1.name, "Shared Name");
    assert_eq!(lab2.name, "Shared Name");
    assert_ne!(lab1.user, lab2.user, "Labs should have different owners");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_lab_invalid_lab_id_too_short() -> Result<()> {
    let db = setup_db("test_create_lab_invalid_short").await?;

    let user = create_user(&db, "grace".to_string(), "TestPass123!", false, vec![]).await?;

    // Try to create lab with lab_id too short
    let result = create_lab(&db, "Invalid Lab", "lab-01", &user, "127.127.1.0/24").await;

    assert!(result.is_err(), "Should fail on invalid lab_id length");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("exactly 8 characters"),
        "Error should mention length requirement, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_lab_invalid_lab_id_too_long() -> Result<()> {
    let db = setup_db("test_create_lab_invalid_long").await?;

    let user = create_user(&db, "heidi".to_string(), "TestPass123!", false, vec![]).await?;

    // Try to create lab with lab_id too long
    let result = create_lab(&db, "Invalid Lab", "lab-00001", &user, "127.127.1.0/24").await;

    assert!(result.is_err(), "Should fail on invalid lab_id length");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("exactly 8 characters"),
        "Error should mention length requirement, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_lab_invalid_lab_id_chars() -> Result<()> {
    let db = setup_db("test_create_lab_invalid_chars").await?;

    let user = create_user(&db, "ivan".to_string(), "TestPass123!", false, vec![]).await?;

    // Try to create lab with invalid characters
    let result = create_lab(&db, "Invalid Lab", "lab_0008", &user, "127.127.1.0/24").await;

    assert!(result.is_err(), "Should fail on invalid characters");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("alphanumeric characters and hyphens"),
        "Error should mention character restrictions, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[test]
fn test_validate_lab_id_valid() {
    assert!(validate_lab_id("lab-0001").is_ok());
    assert!(validate_lab_id("12345678").is_ok());
    assert!(validate_lab_id("abc-defg").is_ok());
    assert!(validate_lab_id("TEST-123").is_ok());
}

#[test]
fn test_validate_lab_id_invalid() {
    assert!(validate_lab_id("short").is_err());
    assert!(validate_lab_id("toolongid").is_err());
    assert!(validate_lab_id("lab_0001").is_err());
    assert!(validate_lab_id("lab 0001").is_err());
}

#[tokio::test]
#[ignore]
async fn test_upsert_lab_creates_new() -> Result<()> {
    let db = setup_db("test_upsert_creates").await?;

    let user = create_user(&db, "judy".to_string(), "TestPass123!", false, vec![]).await?;
    let user_id = user.id.unwrap();

    // Upsert with no ID should create new lab
    let lab = DbLab {
        id: None,
        lab_id: "lab-0009".to_string(),
        name: "Upsert Test".to_string(),
        user: user_id.clone(),
        loopback_network: "127.127.1.0/24".to_string(),
    };

    let result = upsert_lab(&db, lab).await?;

    assert!(result.id.is_some(), "Should have created new lab with ID");
    assert_eq!(result.lab_id, "lab-0009");
    assert_eq!(result.name, "Upsert Test");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_upsert_lab_updates_existing() -> Result<()> {
    let db = setup_db("test_upsert_updates").await?;

    let user = create_user(&db, "karl".to_string(), "TestPass123!", false, vec![]).await?;

    // Create initial lab
    let lab = create_lab(&db, "Original Name", "lab-0010", &user, "127.127.1.0/24").await?;
    let lab_id = lab.id.clone().unwrap();

    // Upsert with ID should update
    let updated_lab = DbLab {
        id: Some(lab_id.clone()),
        lab_id: "lab-0010".to_string(),
        name: "Updated Name".to_string(),
        user: user.id.unwrap(),
        loopback_network: "127.127.1.0/24".to_string(),
    };

    let result = upsert_lab(&db, updated_lab).await?;

    assert_eq!(result.id, Some(lab_id), "ID should remain the same");
    assert_eq!(result.name, "Updated Name", "Name should be updated");

    // Verify via get_lab
    let fetched = get_lab(&db, "lab-0010").await?;
    assert_eq!(fetched.name, "Updated Name");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_lab_increases_count() -> Result<()> {
    let db = setup_db("test_create_increases_count").await?;

    let count_before = count_labs(&db).await?;

    let user = create_user(&db, "laura".to_string(), "TestPass123!", false, vec![]).await?;
    create_lab(&db, "Count Test", "lab-0011", &user, "127.127.1.0/24").await?;

    let count_after = count_labs(&db).await?;

    assert_eq!(count_after, count_before + 1, "Count should increase by 1");

    teardown_db(&db).await?;
    Ok(())
}
