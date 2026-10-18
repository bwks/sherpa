/// CREATE operation tests for user
use anyhow::Result;
use db::{create_admin_user, create_user, get_user, list_users, upsert_user};

use crate::{setup_db, teardown_db};

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_create_user_success() -> Result<()> {
    let db = setup_db("test_create_user_success").await?;

    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;

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

    let user = create_user(
        &db,
        "bob".to_string(),
        "TestPass123!",
        false,
        ssh_keys.clone(),
    )
    .await?;

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
    create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;

    // Try to create user with same username
    let result = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await;

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

    let result = create_user(&db, "ab".to_string(), "TestPass123!", false, vec![]).await;

    assert!(result.is_err(), "Should fail on username too short");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("3–32"),
        "Error should mention the length range: {}",
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
    let result = create_user(&db, "user name".to_string(), "TestPass123!", false, vec![]).await;
    assert!(result.is_err(), "Should fail on space in username");

    // Test special char
    let result = create_user(&db, "user#name".to_string(), "TestPass123!", false, vec![]).await;
    assert!(result.is_err(), "Should fail on # in username");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_user_allows_hyphen_and_underscore() -> Result<()> {
    let db = setup_db("test_create_user_allows_hyphen_and_underscore").await?;

    let user = create_user(
        &db,
        "test-user_01".to_string(),
        "TestPass123!",
        false,
        vec![],
    )
    .await?;
    assert_eq!(user.username, "test-user_01");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_user_rejects_names_invalid_on_guests() -> Result<()> {
    let db = setup_db("test_create_user_rejects_names_invalid_on_guests").await?;

    for username in ["user@example.com", "a.b.c", "1brad", "-brad", "josé"] {
        let result = create_user(&db, username.to_string(), "TestPass123!", false, vec![]).await;
        assert!(result.is_err(), "Should reject '{username}'");
    }

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_user_stores_lowercase_username() -> Result<()> {
    let db = setup_db("test_create_user_stores_lowercase_username").await?;

    let user = create_user(&db, " BRadmin ".to_string(), "TestPass123!", false, vec![]).await?;
    assert_eq!(user.username, "bradmin");

    let found = get_user(&db, "BRADMIN").await?;
    assert_eq!(found.id, user.id);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_user_rejects_case_variant_of_existing_user() -> Result<()> {
    let db = setup_db("test_create_user_rejects_case_variant").await?;

    create_user(&db, "brad".to_string(), "TestPass123!", false, vec![]).await?;
    let result = create_user(&db, "bRad".to_string(), "TestPass123!", false, vec![]).await;
    assert!(
        result.is_err(),
        "A case variant must hit the unique username"
    );
    assert_eq!(list_users(&db).await?.len(), 1);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_user_rejects_reserved_usernames() -> Result<()> {
    let db = setup_db("test_create_user_rejects_reserved_usernames").await?;

    for username in [
        "admin",
        "Admin",
        "sherpa",
        "root",
        "ubuntu",
        "systemd-network",
    ] {
        let error = create_user(&db, username.to_string(), "TestPass123!", false, vec![])
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("reserved"), "'{username}': {error}");
    }
    assert!(list_users(&db).await?.is_empty());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_admin_user_creates_reserved_admin() -> Result<()> {
    let db = setup_db("test_create_admin_user_creates_reserved_admin").await?;

    let admin = create_admin_user(&db, "AdminPass123!").await?;
    assert_eq!(admin.username, "admin");
    assert!(admin.is_admin);
    assert!(admin.ssh_keys.is_empty());

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_schema_rejects_username_written_directly() -> Result<()> {
    let db = setup_db("test_schema_rejects_username_written_directly").await?;

    for username in ["Brad", "brad.searle", "ab", "1brad"] {
        let result = db
            .query(
                "CREATE user CONTENT { username: $username, password_hash: 'x', \
                 created_at: time::now(), updated_at: time::now() }",
            )
            .bind(("username", username))
            .await?
            .check();
        assert!(result.is_err(), "Schema should reject '{username}'");
    }

    // The same statement succeeds for a valid name, so the rejections come from the username.
    db.query(
        "CREATE user CONTENT { username: 'brad', password_hash: 'x', \
         created_at: time::now(), updated_at: time::now() }",
    )
    .await?
    .check()?;

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_upsert_user_creates_new() -> Result<()> {
    let db = setup_db("test_upsert_user_creates").await?;

    let user = upsert_user(
        &db,
        "dave".to_string(),
        "TestPass123!",
        false,
        vec!["key1".to_string()],
    )
    .await?;

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
    let user1 = upsert_user(
        &db,
        "eve".to_string(),
        "TestPass123!",
        false,
        vec!["key1".to_string()],
    )
    .await?;
    assert_eq!(user1.ssh_keys.len(), 1);

    // Upsert with more keys
    let user2 = upsert_user(
        &db,
        "eve".to_string(),
        "TestPass123!",
        false,
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
