use anyhow::Result;
use db::{
    Database, DefaultRequest, LabDetails, create_lab, create_user, delete_lab_from, find_lab,
    list_node_states_by_user, update_lab_phase_from, update_node_state_from, upsert_node_image,
};
use shared::data::{DbLab, DbUser, LabPhase, NodeConfig, NodeModel, NodeState};

use crate::helper::{create_test_node_with_model, ipv4_lab_networks, setup_db};

async fn lab_for(db: &Database, user: &DbUser, lab_id: &str, octet: u8) -> Result<DbLab> {
    create_lab(
        db,
        lab_id,
        lab_id,
        user,
        &ipv4_lab_networks(
            &format!("127.127.{octet}.0/24"),
            &format!("172.31.{octet}.0/24"),
            &format!("172.31.{octet}.1"),
            &format!("172.31.{octet}.2"),
        ),
        &LabDetails::default(),
    )
    .await
}

#[tokio::test]
async fn a_new_lab_is_being_created() -> Result<()> {
    // M3.1: the row `up` inserts is its reservation of the lab ID.
    let db = setup_db().await?;
    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;

    let lab = lab_for(&db, &user, "lab-0001", 1).await?;

    assert_eq!(lab.phase, LabPhase::Creating);
    assert_eq!(
        find_lab(&db, "lab-0001").await?.map(|lab| lab.phase),
        Some(LabPhase::Creating)
    );
    Ok(())
}

#[tokio::test]
async fn a_phase_write_leaves_a_lab_another_writer_moved() -> Result<()> {
    let db = setup_db().await?;
    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    lab_for(&db, &user, "lab-0001", 1).await?;

    let moved = update_lab_phase_from(&db, "lab-0001", LabPhase::Creating, LabPhase::Ready).await?;
    assert_eq!(moved.map(|lab| lab.phase), Some(LabPhase::Ready));

    // A second writer that read `Creating` finds the lab moved on.
    let stale =
        update_lab_phase_from(&db, "lab-0001", LabPhase::Creating, LabPhase::Interrupted).await?;
    assert!(stale.is_none());
    assert_eq!(
        find_lab(&db, "lab-0001").await?.map(|lab| lab.phase),
        Some(LabPhase::Ready)
    );

    // A lab that does not exist is not moved either.
    assert!(
        update_lab_phase_from(&db, "lab-0009", LabPhase::Ready, LabPhase::Destroying)
            .await?
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn a_lab_is_removed_only_from_the_phase_it_was_read_in() -> Result<()> {
    let db = setup_db().await?;
    let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    lab_for(&db, &user, "lab-0001", 1).await?;

    assert!(!delete_lab_from(&db, "lab-0001", LabPhase::Destroying).await?);
    assert!(find_lab(&db, "lab-0001").await?.is_some());

    assert!(delete_lab_from(&db, "lab-0001", LabPhase::Creating).await?);
    assert!(find_lab(&db, "lab-0001").await?.is_none());

    // Removing it again finds nothing, and is not an error.
    assert!(!delete_lab_from(&db, "lab-0001", LabPhase::Creating).await?);
    Ok(())
}

#[tokio::test]
async fn the_node_states_of_a_users_labs_are_read_in_one_query() -> Result<()> {
    let db = setup_db().await?;
    let alice = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let bob = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;
    let first = lab_for(&db, &alice, "lab-0001", 1).await?;
    let second = lab_for(&db, &alice, "lab-0002", 2).await?;
    let others = lab_for(&db, &bob, "lab-0003", 3).await?;
    upsert_node_image(
        &db,
        NodeConfig::get_model(NodeModel::UbuntuLinux),
        DefaultRequest::Keep,
    )
    .await?;
    let a = create_test_node_with_model(&db, "a", 1, NodeModel::UbuntuLinux, &first).await?;
    create_test_node_with_model(&db, "b", 2, NodeModel::UbuntuLinux, &first).await?;
    create_test_node_with_model(&db, "c", 1, NodeModel::UbuntuLinux, &second).await?;
    create_test_node_with_model(&db, "d", 1, NodeModel::UbuntuLinux, &others).await?;
    update_node_state_from(&db, a.id.unwrap(), NodeState::Building, NodeState::Running).await?;

    let mut states = list_node_states_by_user(&db, alice.id.unwrap()).await?;
    states.sort_by_key(|(lab, state)| (lab.clone(), state.to_string()));

    let mut expected = vec![
        (first.id.clone().unwrap(), NodeState::Building),
        (first.id.unwrap(), NodeState::Running),
        (second.id.unwrap(), NodeState::Building),
    ];
    expected.sort_by_key(|(lab, state)| (lab.clone(), state.to_string()));
    assert_eq!(states, expected);
    Ok(())
}
