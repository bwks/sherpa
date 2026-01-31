/// DELETE operation tests for user
use anyhow::Result;
use db::{
    count_users, create_lab, create_user, delete_user, delete_user_by_username, delete_user_safe,
    get_user,
};

use crate::{setup_db, teardown_db};

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_delete_user_success() -> Result<()> {
    let db = setup_db("test_delete_user_success").await?;

    // Create a user
    let user = create_user(&db, "alice".to_string(), vec![]).await?;
    let user_id = user.id.expect("User should have ID");

    // Delete the user
    delete_user(&db, user_id).await?;

    // Verify user is gone
    let result = get_user(&db, "alice").await;
    assert!(result.is_err(), "User should not exist after deletion");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_user_by_username() -> Result<()> {
    let db = setup_db("test_delete_user_by_username").await?;

    create_user(&db, "bob".to_string(), vec![]).await?;

    // Delete by username
    delete_user_by_username(&db, "bob").await?;

    // Verify user is gone
    let result = get_user(&db, "bob").await;
    assert!(result.is_err(), "User should not exist after deletion");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_nonexistent_user_fails() -> Result<()> {
    let db = setup_db("test_delete_nonexistent_user").await?;

    use surrealdb::RecordId;
    let fake_id = RecordId::from(("user", "nonexistent"));

    let result = delete_user(&db, fake_id).await;

    assert!(result.is_err(), "Should fail when user doesn't exist");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found"),
        "Error should mention user not found: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_nonexistent_username_fails() -> Result<()> {
    let db = setup_db("test_delete_nonexistent_username").await?;

    let result = delete_user_by_username(&db, "nonexistent").await;

    assert!(result.is_err(), "Should fail when username doesn't exist");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_user_decreases_count() -> Result<()> {
    let db = setup_db("test_delete_user_count").await?;

    // Create users
    create_user(&db, "charlie".to_string(), vec![]).await?;
    let user2 = create_user(&db, "dave".to_string(), vec![]).await?;

    let count_before = count_users(&db).await?;

    // Delete one user
    delete_user(&db, user2.id.unwrap()).await?;

    let count_after = count_users(&db).await?;

    assert_eq!(count_after, count_before - 1, "Count should decrease by 1");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_user_safe_without_labs() -> Result<()> {
    let db = setup_db("test_delete_user_safe_no_labs").await?;

    let user = create_user(&db, "eve".to_string(), vec![]).await?;
    let user_id = user.id.expect("User should have ID");

    // Should succeed since user has no labs
    delete_user_safe(&db, user_id).await?;

    // Verify user is gone
    let result = get_user(&db, "eve").await;
    assert!(result.is_err(), "User should not exist after deletion");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_user_safe_with_labs_fails() -> Result<()> {
    let db = setup_db("test_delete_user_safe_with_labs").await?;

    // Create user and lab
    let user = create_user(&db, "frank".to_string(), vec![]).await?;
    let user_id = user.id.clone().expect("User should have ID");

    create_lab(&db, "test-lab", "lab-001", &user).await?;

    // Should fail because user owns a lab
    let result = delete_user_safe(&db, user_id).await;

    assert!(result.is_err(), "Should fail when user owns labs");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("owns 1 lab"),
        "Error should mention lab count: {}",
        error_msg
    );
    assert!(
        error_msg.contains("test-lab"),
        "Error should mention lab name: {}",
        error_msg
    );

    // Verify user still exists
    let user = get_user(&db, "frank").await?;
    assert_eq!(user.username, "frank");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_user_safe_with_multiple_labs_fails() -> Result<()> {
    let db = setup_db("test_delete_user_safe_multiple_labs").await?;

    // Create user with multiple labs
    let user = create_user(&db, "grace".to_string(), vec![]).await?;
    let user_id = user.id.clone().expect("User should have ID");

    create_lab(&db, "lab-1", "lab-001", &user).await?;
    create_lab(&db, "lab-2", "lab-002", &user).await?;
    create_lab(&db, "lab-3", "lab-003", &user).await?;

    // Should fail with count of 3 labs
    let result = delete_user_safe(&db, user_id).await;

    assert!(result.is_err(), "Should fail when user owns multiple labs");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("owns 3 lab"),
        "Error should mention 3 labs: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_user_with_labs_needs_manual_cleanup() -> Result<()> {
    let db = setup_db("test_delete_user_manual_cleanup").await?;

    use db::delete_lab;

    // Create user and lab
    let user = create_user(&db, "heidi".to_string(), vec![]).await?;
    let user_id = user.id.clone().expect("User should have ID");

    let lab = create_lab(&db, "test-lab", "lab-001", &user).await?;

    // Without cascade delete in schema, we must manually delete labs first
    delete_lab(&db, &lab.lab_id).await?;

    // Now user can be deleted
    delete_user(&db, user_id).await?;

    // Verify user is gone
    let user_result = get_user(&db, "heidi").await;
    assert!(user_result.is_err(), "User should be deleted");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_delete_user_safe_nonexistent_fails() -> Result<()> {
    let db = setup_db("test_delete_user_safe_nonexistent").await?;

    use surrealdb::RecordId;
    let fake_id = RecordId::from(("user", "nonexistent"));

    let result = delete_user_safe(&db, fake_id).await;

    assert!(result.is_err(), "Should fail when user doesn't exist");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found"),
        "Error should mention user not found: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}
