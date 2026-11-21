/// DELETE operation tests for user
use anyhow::Result;
use db::delete_user;
use shared::data::RecordId;

use crate::setup_db;

#[tokio::test]
async fn user_accounts_017_deleting_a_user_that_does_not_exist_is_not_found() -> Result<()> {
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
