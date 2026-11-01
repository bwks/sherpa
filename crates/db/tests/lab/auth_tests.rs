/// Authentication/authorization tests for lab operations
use anyhow::Result;
use db::{create_lab, create_user, get_lab_owner_username};

use crate::{ipv4_lab_networks, setup_db};

#[tokio::test]
async fn test_get_lab_owner_username() -> Result<()> {
    let db = setup_db().await?;

    // Create a user
    let user = create_user(&db, "testuser".to_string(), "TestPass123!", false, vec![]).await?;

    // Create a lab owned by this user
    let lab = create_lab(
        &db,
        "Test Lab",
        "lab-0001",
        &user,
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
    )
    .await?;

    // Get the owner username
    let username = get_lab_owner_username(&db, &lab.lab_id).await?;
    assert_eq!(username, "testuser");

    Ok(())
}

#[tokio::test]
async fn test_get_lab_owner_username_not_found() -> Result<()> {
    let db = setup_db().await?;

    // Try to get owner of non-existent lab
    let result = get_lab_owner_username(&db, "no-exist").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));

    Ok(())
}
