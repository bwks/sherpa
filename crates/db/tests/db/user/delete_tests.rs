/// DELETE operation tests for user
use anyhow::Result;
use db::{DbError, delete_user};
use shared::data::RecordId;

use crate::setup_db;

#[tokio::test]
async fn user_accounts_017_deleting_a_user_that_does_not_exist_is_not_found() -> Result<()> {
    let db = setup_db().await?;

    let fake_id = RecordId::new("user", "nonexistent");

    let error = delete_user(&db, fake_id.clone()).await.unwrap_err();

    assert_eq!(
        error.downcast_ref::<DbError>(),
        Some(&DbError::UserToDeleteNotFound { id: fake_id }),
        "{error:#}"
    );

    Ok(())
}
