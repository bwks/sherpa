use anyhow::Result;
use db::{LabDetails, count_labs_by_user, create_lab, create_user, find_lab, get_lab, list_labs};

use crate::helper::{ipv4_lab_networks, setup_db};

#[tokio::test]
async fn test_get_lab_by_lab_id() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let created = create_lab(
        &db,
        "Test Lab",
        "lab-0001",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;

    let fetched = get_lab(&db, "lab-0001").await?;

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.name, "Test Lab");
    assert_eq!(fetched.lab_id, "lab-0001");

    Ok(())
}

#[tokio::test]
async fn test_list_labs_returns_all_fields() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "heidi".to_string(), "TestPass123!", false, vec![]).await?;
    let user_id = user.id.as_ref().unwrap().clone();
    create_lab(
        &db,
        "Complete Lab",
        "lab-0010",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;

    let labs = list_labs(&db).await?;

    assert_eq!(labs.len(), 1);
    let lab = &labs[0];

    assert!(lab.id.is_some(), "Should have ID");
    assert_eq!(lab.name, "Complete Lab");
    assert_eq!(lab.lab_id, "lab-0010");
    assert_eq!(lab.user, user_id);

    Ok(())
}

#[tokio::test]
async fn test_count_labs_by_user() -> Result<()> {
    let db = setup_db().await?;

    let user1 = create_user(&db, "judy".to_string(), "TestPass123!", false, vec![]).await?;
    let user2 = create_user(&db, "karl".to_string(), "TestPass123!", false, vec![]).await?;

    create_lab(
        &db,
        "Judy Lab 1",
        "lab-0013",
        &user1,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    create_lab(
        &db,
        "Judy Lab 2",
        "lab-0014",
        &user1,
        &ipv4_lab_networks("172.31.2.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    create_lab(
        &db,
        "Judy Lab 3",
        "lab-0015",
        &user1,
        &ipv4_lab_networks("172.31.3.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    create_lab(
        &db,
        "Karl Lab",
        "lab-0016",
        &user2,
        &ipv4_lab_networks("172.31.4.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;

    let user1_count = count_labs_by_user(&db, user1.id.unwrap()).await?;
    let user2_count = count_labs_by_user(&db, user2.id.unwrap()).await?;

    assert_eq!(user1_count, 3, "User1 should have 3 labs");
    assert_eq!(user2_count, 1, "User2 should have 1 lab");

    Ok(())
}

#[tokio::test]
async fn find_lab_returns_none_for_a_missing_lab() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let created = create_lab(
        &db,
        "Test Lab",
        "lab-0001",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;

    let found = find_lab(&db, "lab-0001").await?.expect("lab should exist");
    assert_eq!(found.id, created.id);
    assert!(find_lab(&db, "invalid").await?.is_none());

    Ok(())
}
