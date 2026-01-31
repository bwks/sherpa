/// CREATE operation tests for user
use anyhow::Result;
use db::{create_user, get_user, list_users, upsert_user};

use crate::{setup_db, teardown_db};

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_create_user_success() -> Result<()> {
    let db = setup_db("test_create_user_success").await?;

    let user = create_user(&db, "alice".to_string(), vec![]).await?;

    assert_eq!(user.username, "alice");
    assert!(user.id.is_some(), "Created user should have an ID");
    assert_eq!(user.ssh_keys.len(), 0);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_user_with_ssh_keys() -> Result<()> {
    let db = setup_db("test_create_user_with_ssh_keys").await?;

    let ssh_keys = vec![
        "ssh-rsa AAAAB3NzaC1yc2EAAA...".to_string(),
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5...".to_string(),
    ];

    let user = create_user(&db, "bob".to_string(), ssh_keys.clone()).await?;

    assert_eq!(user.username, "bob");
    assert_eq!(user.ssh_keys, ssh_keys);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_user_duplicate_username_fails() -> Result<()> {
    let db = setup_db("test_create_user_duplicate_username").await?;

    // Create first user
    create_user(&db, "charlie".to_string(), vec![]).await?;

    // Try to create user with same username
    let result = create_user(&db, "charlie".to_string(), vec![]).await;

    assert!(result.is_err(), "Should fail on duplicate username");
    let error_msg = result.unwrap_err().to_string();
    
    // Verify the error is about creating the user (the underlying DB error may be masked by context)
    println!("Duplicate username error: {}", error_msg);
    assert!(
        error_msg.contains("Failed to create user") && error_msg.contains("charlie"),
        "Error should mention the failed user creation, got: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_user_username_too_short() -> Result<()> {
    let db = setup_db("test_create_user_username_too_short").await?;

    let result = create_user(&db, "ab".to_string(), vec![]).await;

    assert!(result.is_err(), "Should fail on username too short");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("at least 3"),
        "Error should mention minimum length: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_user_invalid_username_chars() -> Result<()> {
    let db = setup_db("test_create_user_invalid_username").await?;

    // Test space
    let result = create_user(&db, "user name".to_string(), vec![]).await;
    assert!(result.is_err(), "Should fail on space in username");

    // Test special char
    let result = create_user(&db, "user#name".to_string(), vec![]).await;
    assert!(result.is_err(), "Should fail on # in username");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_user_valid_special_chars() -> Result<()> {
    let db = setup_db("test_create_user_valid_special_chars").await?;

    // Test allowed special characters: @ . _ -
    let user1 = create_user(&db, "user@example.com".to_string(), vec![]).await?;
    assert_eq!(user1.username, "user@example.com");

    let user2 = create_user(&db, "test-user_01".to_string(), vec![]).await?;
    assert_eq!(user2.username, "test-user_01");

    let user3 = create_user(&db, "a.b.c".to_string(), vec![]).await?;
    assert_eq!(user3.username, "a.b.c");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_upsert_user_creates_new() -> Result<()> {
    let db = setup_db("test_upsert_user_creates").await?;

    let user = upsert_user(&db, "dave".to_string(), vec!["key1".to_string()]).await?;

    assert_eq!(user.username, "dave");
    assert_eq!(user.ssh_keys.len(), 1);

    // Verify it exists
    let fetched = get_user(&db, "dave").await?;
    assert_eq!(fetched.username, "dave");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_upsert_user_updates_existing() -> Result<()> {
    let db = setup_db("test_upsert_user_updates").await?;

    // Create initial user
    let user1 = upsert_user(&db, "eve".to_string(), vec!["key1".to_string()]).await?;
    assert_eq!(user1.ssh_keys.len(), 1);

    // Upsert with more keys
    let user2 = upsert_user(
        &db,
        "eve".to_string(),
        vec!["key1".to_string(), "key2".to_string()],
    )
    .await?;
    assert_eq!(user2.ssh_keys.len(), 2);
    assert_eq!(user2.username, "eve");

    // Verify only one user exists
    let all_users = list_users(&db).await?;
    let eve_users: Vec<_> = all_users.iter().filter(|u| u.username == "eve").collect();
    assert_eq!(eve_users.len(), 1, "Should only have one 'eve' user");

    teardown_db(&db).await?;
    Ok(())
}
