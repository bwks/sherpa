/// DELETE operation tests for user
use anyhow::Result;
use db::{create_user, delete_user, get_user};
use shared::data::RecordId;

use crate::setup_db;

#[tokio::test]
async fn test_delete_user_success() -> Result<()> {
    let db = setup_db().await?;

    // Create a user
    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let user_id = user.id.expect("User should have ID");

    // Delete the user
    delete_user(&db, user_id).await?;

    // Verify user is gone
    let result = get_user(&db, "alice").await;
    assert!(result.is_err(), "User should not exist after deletion");

    Ok(())
}

#[tokio::test]
async fn test_delete_nonexistent_user_fails() -> Result<()> {
    let db = setup_db().await?;

    let fake_id = RecordId::new("user", "nonexistent");

    let result = delete_user(&db, fake_id).await;

    assert!(result.is_err(), "Should fail when user doesn't exist");
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found"),
        "Error should mention user not found: {error_msg}"
    );

    Ok(())
}
