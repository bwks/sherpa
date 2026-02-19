/// UPDATE operation tests for user
use anyhow::Result;
use db::{create_user, get_user, update_user};

use crate::{setup_db, teardown_db};

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_update_user_success() -> Result<()> {
    let db = setup_db("test_update_user_success").await?;

    // Create a user
    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;

    // Update the user
    let mut updated_user = user.clone();
    updated_user.ssh_keys = vec!["new-key-1".to_string(), "new-key-2".to_string()];

    let result = update_user(&db, updated_user).await?;

    assert_eq!(result.username, "alice");
    assert_eq!(result.ssh_keys.len(), 2);
    assert_eq!(result.ssh_keys[0], "new-key-1");

    // Verify update persisted
    let fetched = get_user(&db, "alice").await?;
    assert_eq!(fetched.ssh_keys.len(), 2);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_user_ssh_keys() -> Result<()> {
    let db = setup_db("test_update_user_ssh_keys").await?;

    let initial_keys = vec!["key1".to_string()];
    let user = create_user(&db, "bob".to_string(), "TestPass123!", false, initial_keys).await?;

    // Add more keys
    let mut updated_user = user.clone();
    updated_user.ssh_keys = vec!["key1".to_string(), "key2".to_string(), "key3".to_string()];

    let result = update_user(&db, updated_user).await?;
    assert_eq!(result.ssh_keys.len(), 3);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_user_remove_all_ssh_keys() -> Result<()> {
    let db = setup_db("test_update_user_remove_keys").await?;

    let user = create_user(
        &db,
        "charlie".to_string(),
        "TestPass123!",
        false,
        vec!["key1".to_string()],
    )
    .await?;

    // Remove all keys
    let mut updated_user = user.clone();
    updated_user.ssh_keys = vec![];

    let result = update_user(&db, updated_user).await?;
    assert_eq!(result.ssh_keys.len(), 0);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_user_without_id_fails() -> Result<()> {
    let db = setup_db("test_update_user_no_id").await?;

    let user = create_user(&db, "dave".to_string(), "TestPass123!", false, vec![]).await?;

    // Create user without ID
    let mut user_no_id = user.clone();
    user_no_id.id = None;

    let result = update_user(&db, user_no_id).await;

    assert!(result.is_err(), "Should fail when user has no ID");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("no ID"),
        "Error should mention missing ID: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_nonexistent_user_fails() -> Result<()> {
    let db = setup_db("test_update_nonexistent_user").await?;

    use shared::data::DbUser;
    use surrealdb_types::RecordId;
    use surrealdb_types::Datetime;

    let fake_user = DbUser {
        id: Some(RecordId::new("user", "nonexistent")),
        username: "fake".to_string(),
        password_hash: "$argon2id$v=19$m=19456,t=2,p=1$test$test".to_string(),
        is_admin: false,
        ssh_keys: vec![],
        created_at: Datetime::default(),
        updated_at: Datetime::default(),
    };

    let result = update_user(&db, fake_user).await;

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
async fn test_update_user_preserves_id() -> Result<()> {
    let db = setup_db("test_update_user_preserves_id").await?;

    let user = create_user(&db, "eve".to_string(), "TestPass123!", false, vec![]).await?;
    let original_id = user.id.clone().unwrap();

    // Update user
    let mut updated_user = user.clone();
    updated_user.ssh_keys = vec!["new-key".to_string()];

    let result = update_user(&db, updated_user).await?;

    assert_eq!(result.id, Some(original_id), "ID should be preserved");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_username() -> Result<()> {
    let db = setup_db("test_update_username").await?;

    let user = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;

    // Update username
    let mut updated_user = user.clone();
    updated_user.username = "franklin".to_string();

    let result = update_user(&db, updated_user).await?;

    assert_eq!(result.username, "franklin");

    // Verify old username doesn't exist
    let old_result = get_user(&db, "frank").await;
    assert!(old_result.is_err(), "Old username should not exist");

    // Verify new username exists
    let new_user = get_user(&db, "franklin").await?;
    assert_eq!(new_user.username, "franklin");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_update_username_conflict_fails() -> Result<()> {
    let db = setup_db("test_update_username_conflict").await?;

    // Create two users
    create_user(&db, "grace".to_string(), "TestPass123!", false, vec![]).await?;
    let user2 = create_user(&db, "heidi".to_string(), "TestPass123!", false, vec![]).await?;

    // Try to rename user2 to user1's name
    let mut updated_user = user2.clone();
    updated_user.username = "grace".to_string();

    let result = update_user(&db, updated_user).await;

    assert!(
        result.is_err(),
        "Should fail when username conflicts with existing user"
    );

    teardown_db(&db).await?;
    Ok(())
}
