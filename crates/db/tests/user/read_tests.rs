/// READ operation tests for user
use anyhow::Result;
use db::{count_users, create_user, get_user, get_user_by_id, list_users};

use crate::{setup_db, teardown_db};

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_get_user_by_username() -> Result<()> {
    let db = setup_db("test_get_user_by_username").await?;

    // Create a user
    create_user(&db, "alice".to_string(), vec!["key1".to_string()]).await?;

    // Get the user
    let user = get_user(&db, "alice").await?;

    assert_eq!(user.username, "alice");
    assert_eq!(user.ssh_keys.len(), 1);
    assert!(user.id.is_some());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_user_not_found() -> Result<()> {
    let db = setup_db("test_get_user_not_found").await?;

    let result = get_user(&db, "nonexistent").await;

    assert!(result.is_err(), "Should fail when user not found");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found") || error_msg.contains("nonexistent"),
        "Error should mention user not found: {}",
        error_msg
    );

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_user_by_id() -> Result<()> {
    let db = setup_db("test_get_user_by_id").await?;

    // Create a user
    let created = create_user(&db, "bob".to_string(), vec![]).await?;
    let user_id = created.id.expect("User should have ID");

    // Get by ID
    let user = get_user_by_id(&db, user_id).await?;

    assert!(user.is_some(), "Should find user by ID");
    let user = user.unwrap();
    assert_eq!(user.username, "bob");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_user_by_id_not_found() -> Result<()> {
    let db = setup_db("test_get_user_by_id_not_found").await?;

    use surrealdb::RecordId;
    let fake_id = RecordId::from(("user", "nonexistent"));

    let user = get_user_by_id(&db, fake_id).await?;

    assert!(user.is_none(), "Should return None for nonexistent ID");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_users() -> Result<()> {
    let db = setup_db("test_list_users").await?;

    // Create multiple users
    create_user(&db, "alice".to_string(), vec![]).await?;
    create_user(&db, "bob".to_string(), vec![]).await?;
    create_user(&db, "charlie".to_string(), vec![]).await?;

    let users = list_users(&db).await?;

    assert!(users.len() >= 3, "Should have at least 3 users");

    let usernames: Vec<String> = users.iter().map(|u| u.username.clone()).collect();
    assert!(usernames.contains(&"alice".to_string()));
    assert!(usernames.contains(&"bob".to_string()));
    assert!(usernames.contains(&"charlie".to_string()));

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_users_returns_all_fields() -> Result<()> {
    let db = setup_db("test_list_users_all_fields").await?;

    let ssh_keys = vec!["key1".to_string(), "key2".to_string()];
    create_user(&db, "dave".to_string(), ssh_keys.clone()).await?;

    let users = list_users(&db).await?;
    let dave = users.iter().find(|u| u.username == "dave").unwrap();

    assert_eq!(dave.username, "dave");
    assert_eq!(dave.ssh_keys, ssh_keys);
    assert!(dave.id.is_some());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_users() -> Result<()> {
    let db = setup_db("test_count_users").await?;

    let initial_count = count_users(&db).await?;

    // Create users
    create_user(&db, "alice".to_string(), vec![]).await?;
    create_user(&db, "bob".to_string(), vec![]).await?;

    let new_count = count_users(&db).await?;

    assert_eq!(new_count, initial_count + 2, "Count should increase by 2");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_users_empty() -> Result<()> {
    let db = setup_db("test_count_users_empty").await?;

    let count = count_users(&db).await?;

    assert_eq!(count, 0, "Should have 0 users in fresh database");

    teardown_db(&db).await?;
    Ok(())
}
