/// Authentication/authorization tests for lab operations
use anyhow::Result;
use db::{create_lab, create_user, get_lab_owner_username};

use crate::{setup_db, teardown_db};

#[tokio::test]
#[ignore] // Requires running SurrealDB instance
async fn test_get_lab_owner_username() -> Result<()> {
    let db = setup_db("test_get_lab_owner_username").await?;

    // Create a user
    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;

    // Create a lab owned by this user
    let lab = create_lab(&db, "Test Lab", "lab-0001", &user).await?;

    // Get the owner username
    let username = get_lab_owner_username(&db, &lab.lab_id).await?;
    assert_eq!(username, "testuser");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_lab_owner_username_not_found() -> Result<()> {
    let db = setup_db("test_get_lab_owner_username_not_found").await?;

    // Try to get owner of non-existent lab
    let result = get_lab_owner_username(&db, "no-exist").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Lab not found"));

    teardown_db(&db).await?;
    Ok(())
}
