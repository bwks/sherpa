/// UPDATE operation tests for user
use anyhow::Result;
use db::{UserChanged, create_user, get_user, update_user};

use crate::setup_db;

/// Two edits read the same row; the second write must not overwrite the first (M14.5).
#[tokio::test]
async fn test_update_from_a_stale_read_is_refused() -> Result<()> {
    let db = setup_db().await?;
    let user = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;

    let mut first = user.clone();
    first.ssh_keys = vec!["key-from-first-edit".to_string()];
    let mut second = user.clone();
    second.ssh_keys = vec!["key-from-second-edit".to_string()];

    update_user(&db, first).await?;
    let error = update_user(&db, second).await.unwrap_err();

    assert!(
        error.downcast_ref::<UserChanged>().is_some(),
        "A stale write should fail with UserChanged: {error:?}"
    );
    let stored = get_user(&db, "frank").await?;
    assert_eq!(stored.ssh_keys, vec!["key-from-first-edit".to_string()]);
    Ok(())
}

#[tokio::test]
async fn test_update_from_a_fresh_read_after_a_change_succeeds() -> Result<()> {
    let db = setup_db().await?;
    let user = create_user(&db, "grace".to_string(), "TestPass123!", false, vec![]).await?;
    let mut first = user.clone();
    first.ssh_keys = vec!["one".to_string()];
    update_user(&db, first).await?;

    let mut reread = get_user(&db, "grace").await?;
    reread.ssh_keys.push("two".to_string());
    let stored = update_user(&db, reread).await?;

    assert_eq!(stored.ssh_keys, vec!["one".to_string(), "two".to_string()]);
    Ok(())
}
