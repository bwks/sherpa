/// READ operation tests for user
use anyhow::Result;
use db::{count_users, create_user, find_user, get_user, get_user_by_id, list_users};
use shared::data::RecordId;

use crate::setup_db;

#[tokio::test]
async fn test_get_user_by_username() -> Result<()> {
    let db = setup_db().await?;

    // Create a user
    create_user(
        &db,
        "alice".to_string(),
        "TestPass123!",
        false,
        vec!["key1".to_string()],
    )
    .await?;

    // Get the user
    let user = get_user(&db, "alice").await?;

    assert_eq!(user.username, "alice");
    assert_eq!(user.ssh_keys.len(), 1);
    assert!(user.id.is_some());

    Ok(())
}

#[tokio::test]
async fn test_get_user_not_found() -> Result<()> {
    let db = setup_db().await?;

    let result = get_user(&db, "nonexistent").await;

    assert!(result.is_err(), "Should fail when user not found");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found") || error_msg.contains("nonexistent"),
        "Error should mention user not found: {}",
        error_msg
    );

    Ok(())
}

#[tokio::test]
async fn test_get_user_by_id() -> Result<()> {
    let db = setup_db().await?;

    // Create a user
    let created = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    let user_id = created.id.expect("User should have ID");

    // Get by ID
    let user = get_user_by_id(&db, user_id).await?;

    assert!(user.is_some(), "Should find user by ID");
    let user = user.unwrap();
    assert_eq!(user.username, "bob");

    Ok(())
}

#[tokio::test]
async fn test_get_user_by_id_not_found() -> Result<()> {
    let db = setup_db().await?;

    let fake_id = RecordId::new("user", "nonexistent");

    let user = get_user_by_id(&db, fake_id).await?;

    assert!(user.is_none(), "Should return None for nonexistent ID");

    Ok(())
}

#[tokio::test]
async fn test_list_users() -> Result<()> {
    let db = setup_db().await?;

    // Create multiple users
    create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;

    let users = list_users(&db).await?;

    assert!(users.len() >= 3, "Should have at least 3 users");

    let usernames: Vec<String> = users.iter().map(|u| u.username.clone()).collect();
    assert!(usernames.contains(&"alice".to_string()));
    assert!(usernames.contains(&"bob".to_string()));
    assert!(usernames.contains(&"charlie".to_string()));

    Ok(())
}

#[tokio::test]
async fn test_list_users_returns_all_fields() -> Result<()> {
    let db = setup_db().await?;

    let ssh_keys = vec!["key1".to_string(), "key2".to_string()];
    create_user(
        &db,
        "dave".to_string(),
        "TestPass123!",
        false,
        ssh_keys.clone(),
    )
    .await?;

    let users = list_users(&db).await?;
    let dave = users.iter().find(|u| u.username == "dave").unwrap();

    assert_eq!(dave.username, "dave");
    assert_eq!(dave.ssh_keys, ssh_keys);
    assert!(dave.id.is_some());

    Ok(())
}

#[tokio::test]
async fn test_count_users() -> Result<()> {
    let db = setup_db().await?;

    let initial_count = count_users(&db).await?;

    // Create users
    create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;

    let new_count = count_users(&db).await?;

    assert_eq!(new_count, initial_count + 2, "Count should increase by 2");

    Ok(())
}

#[tokio::test]
async fn test_count_users_empty() -> Result<()> {
    let db = setup_db().await?;

    let count = count_users(&db).await?;

    assert_eq!(count, 0, "Should have 0 users in fresh database");

    Ok(())
}

#[tokio::test]
async fn find_user_returns_none_for_a_missing_user() -> Result<()> {
    let db = setup_db().await?;

    create_user(&db, "alice".to_string(), "TestPass123!", true, vec![]).await?;

    let found = find_user(&db, "alice").await?.expect("alice should exist");
    assert_eq!(found.username, "alice");
    assert!(found.is_admin);
    assert!(find_user(&db, "nonexistent").await?.is_none());

    Ok(())
}
