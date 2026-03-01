use anyhow::Result;
use db::{
    count_labs, count_labs_by_user, create_lab, create_user, get_lab, get_lab_by_id,
    get_lab_by_name_and_user, get_used_management_networks, list_labs, list_labs_by_user,
};
use shared::data::RecordId;

use crate::helper::{setup_db, teardown_db};

#[tokio::test]
#[ignore]
async fn test_get_lab_by_lab_id() -> Result<()> {
    let db = setup_db("test_get_lab").await?;

    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let created = create_lab(
        &db,
        "Test Lab",
        "lab-0001",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;

    let fetched = get_lab(&db, "lab-0001").await?;

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.name, "Test Lab");
    assert_eq!(fetched.lab_id, "lab-0001");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_lab_not_found() -> Result<()> {
    let db = setup_db("test_get_lab_not_found").await?;

    let result = get_lab(&db, "invalid").await;

    assert!(result.is_err(), "Should error on non-existent lab");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_lab_by_id() -> Result<()> {
    let db = setup_db("test_get_lab_by_id").await?;

    let user = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    let created = create_lab(
        &db,
        "Test Lab",
        "lab-0002",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    let lab_id = created.id.clone().unwrap();

    let fetched = get_lab_by_id(&db, lab_id).await?;

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.name, "Test Lab");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_lab_by_id_not_found() -> Result<()> {
    let db = setup_db("test_get_lab_by_id_not_found").await?;

    let fake_id = RecordId::new("lab", "nonexistent");
    let result = get_lab_by_id(&db, fake_id).await;

    assert!(result.is_err(), "Should error on non-existent lab ID");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_lab_by_name_and_user() -> Result<()> {
    let db = setup_db("test_get_lab_by_name_user").await?;

    let user = create_user(&db, "charlie".to_string(), "TestPass123!", false, vec![]).await?;
    let user_id = user.id.as_ref().unwrap().clone();

    let created = create_lab(
        &db,
        "My Lab",
        "lab-0003",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;

    let fetched = get_lab_by_name_and_user(&db, "My Lab", user_id).await?;

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.name, "My Lab");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_lab_by_name_and_user_not_found() -> Result<()> {
    let db = setup_db("test_get_lab_name_user_not_found").await?;

    let user = create_user(&db, "diana".to_string(), "TestPass123!", false, vec![]).await?;
    let user_id = user.id.unwrap();

    let result = get_lab_by_name_and_user(&db, "Nonexistent", user_id).await;

    assert!(result.is_err(), "Should error when lab not found");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_labs() -> Result<()> {
    let db = setup_db("test_list_labs").await?;

    let user = create_user(&db, "eve".to_string(), "TestPass123!", false, vec![]).await?;

    create_lab(
        &db,
        "Lab 1",
        "lab-0004",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    create_lab(
        &db,
        "Lab 2",
        "lab-0005",
        &user,
        "127.127.2.0/24",
        "172.31.2.0/24",
    )
    .await?;
    create_lab(
        &db,
        "Lab 3",
        "lab-0006",
        &user,
        "127.127.3.0/24",
        "172.31.3.0/24",
    )
    .await?;

    let labs = list_labs(&db).await?;

    assert_eq!(labs.len(), 3, "Should return all 3 labs");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_labs_empty() -> Result<()> {
    let db = setup_db("test_list_labs_empty").await?;

    let labs = list_labs(&db).await?;

    assert_eq!(labs.len(), 0, "Should return empty list");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_labs_by_user() -> Result<()> {
    let db = setup_db("test_list_labs_by_user").await?;

    let user1 = create_user(&db, "frank".to_string(), "TestPass123!", false, vec![]).await?;
    let user2 = create_user(&db, "grace".to_string(), "TestPass123!", false, vec![]).await?;

    create_lab(
        &db,
        "Frank Lab 1",
        "lab-0007",
        &user1,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    create_lab(
        &db,
        "Frank Lab 2",
        "lab-0008",
        &user1,
        "127.127.2.0/24",
        "172.31.2.0/24",
    )
    .await?;
    create_lab(
        &db,
        "Grace Lab",
        "lab-0009",
        &user2,
        "127.127.3.0/24",
        "172.31.3.0/24",
    )
    .await?;

    let user1_labs = list_labs_by_user(&db, user1.id.unwrap()).await?;
    let user2_labs = list_labs_by_user(&db, user2.id.unwrap()).await?;

    assert_eq!(user1_labs.len(), 2, "User1 should have 2 labs");
    assert_eq!(user2_labs.len(), 1, "User2 should have 1 lab");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_list_labs_returns_all_fields() -> Result<()> {
    let db = setup_db("test_list_labs_fields").await?;

    let user = create_user(&db, "heidi".to_string(), "TestPass123!", false, vec![]).await?;
    let user_id = user.id.as_ref().unwrap().clone();
    create_lab(
        &db,
        "Complete Lab",
        "lab-0010",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;

    let labs = list_labs(&db).await?;

    assert_eq!(labs.len(), 1);
    let lab = &labs[0];

    assert!(lab.id.is_some(), "Should have ID");
    assert_eq!(lab.name, "Complete Lab");
    assert_eq!(lab.lab_id, "lab-0010");
    assert_eq!(lab.user, user_id);

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_labs() -> Result<()> {
    let db = setup_db("test_count_labs").await?;

    let user = create_user(&db, "ivan".to_string(), "TestPass123!", false, vec![]).await?;

    let count_before = count_labs(&db).await?;
    assert_eq!(count_before, 0, "Should start with 0 labs");

    create_lab(
        &db,
        "Lab 1",
        "lab-0011",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    create_lab(
        &db,
        "Lab 2",
        "lab-0012",
        &user,
        "127.127.2.0/24",
        "172.31.2.0/24",
    )
    .await?;

    let count_after = count_labs(&db).await?;
    assert_eq!(count_after, 2, "Should have 2 labs");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_count_labs_by_user() -> Result<()> {
    let db = setup_db("test_count_labs_by_user").await?;

    let user1 = create_user(&db, "judy".to_string(), "TestPass123!", false, vec![]).await?;
    let user2 = create_user(&db, "karl".to_string(), "TestPass123!", false, vec![]).await?;

    create_lab(
        &db,
        "Judy Lab 1",
        "lab-0013",
        &user1,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    create_lab(
        &db,
        "Judy Lab 2",
        "lab-0014",
        &user1,
        "127.127.2.0/24",
        "172.31.2.0/24",
    )
    .await?;
    create_lab(
        &db,
        "Judy Lab 3",
        "lab-0015",
        &user1,
        "127.127.3.0/24",
        "172.31.3.0/24",
    )
    .await?;
    create_lab(
        &db,
        "Karl Lab",
        "lab-0016",
        &user2,
        "127.127.4.0/24",
        "172.31.4.0/24",
    )
    .await?;

    let user1_count = count_labs_by_user(&db, user1.id.unwrap()).await?;
    let user2_count = count_labs_by_user(&db, user2.id.unwrap()).await?;

    assert_eq!(user1_count, 3, "User1 should have 3 labs");
    assert_eq!(user2_count, 1, "User2 should have 1 lab");

    teardown_db(&db).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_used_management_networks() -> Result<()> {
    let db = setup_db("test_used_mgmt_nets").await?;

    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;

    // No labs yet — should return empty
    let used = get_used_management_networks(&db).await?;
    assert!(used.is_empty(), "Should be empty when no labs exist");

    // Create two labs with different management networks
    create_lab(
        &db,
        "Lab A",
        "lab-0017",
        &user,
        "127.127.1.0/24",
        "172.31.1.0/24",
    )
    .await?;
    create_lab(
        &db,
        "Lab B",
        "lab-0018",
        &user,
        "127.127.2.0/24",
        "172.31.2.0/24",
    )
    .await?;

    let used = get_used_management_networks(&db).await?;
    assert_eq!(used.len(), 2, "Should have 2 management networks");

    let net1: ipnet::Ipv4Net = "172.31.1.0/24".parse()?;
    let net2: ipnet::Ipv4Net = "172.31.2.0/24".parse()?;
    assert!(used.contains(&net1), "Should contain 172.31.1.0/24");
    assert!(used.contains(&net2), "Should contain 172.31.2.0/24");

    teardown_db(&db).await?;
    Ok(())
}
