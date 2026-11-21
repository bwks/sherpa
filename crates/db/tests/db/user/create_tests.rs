/// CREATE operation tests for user
use anyhow::Result;
use db::{create_user, get_user, list_users};

use crate::setup_db;

#[tokio::test]
async fn test_create_user_duplicate_username_fails() -> Result<()> {
    let db = setup_db().await?;

    // Create first user
    create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;

    // Try to create user with same username
    let result = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await;

    assert!(result.is_err(), "Should fail on duplicate username");
    let error_msg = result.unwrap_err().to_string();

    // Verify the error is about creating the user (the underlying DB error may be masked by context)
    println!("Duplicate username error: {error_msg}");
    assert!(
        error_msg.contains("Failed to create user") && error_msg.contains("charlie"),
        "Error should mention the failed user creation, got: {error_msg}"
    );

    Ok(())
}

#[tokio::test]
async fn test_create_user_username_too_short() -> Result<()> {
    let db = setup_db().await?;

    let result = create_user(&db, "ab".to_string(), "TestPass123!", false, vec![]).await;

    assert!(result.is_err(), "Should fail on username too short");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("3–32"),
        "Error should mention the length range: {error_msg}"
    );

    Ok(())
}

#[tokio::test]
async fn test_create_user_stores_lowercase_username() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, " BRadmin ".to_string(), "TestPass123!", false, vec![]).await?;
    assert_eq!(user.username, "bradmin");

    let found = get_user(&db, "BRADMIN").await?;
    assert_eq!(found.id, user.id);

    Ok(())
}

#[tokio::test]
async fn test_create_user_rejects_case_variant_of_existing_user() -> Result<()> {
    let db = setup_db().await?;

    create_user(&db, "brad".to_string(), "TestPass123!", false, vec![]).await?;
    let result = create_user(&db, "bRad".to_string(), "TestPass123!", false, vec![]).await;
    assert!(
        result.is_err(),
        "A case variant must hit the unique username"
    );
    assert_eq!(list_users(&db).await?.len(), 1);

    Ok(())
}

#[tokio::test]
async fn test_create_user_rejects_reserved_usernames() -> Result<()> {
    let db = setup_db().await?;

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

    Ok(())
}
