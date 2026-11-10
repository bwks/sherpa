use anyhow::Result;
use db::{
    LabDetails, create_lab, create_node, create_user, get_node, update_node_state_from,
    upsert_node_image,
};
use shared::data::{DbNode, NodeConfig, NodeModel, NodeState, RecordId};

use crate::helper::{ipv4_lab_networks, setup_db};

async fn one_node(db: &db::Database) -> Result<DbNode> {
    let user = create_user(db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let lab = create_lab(
        db,
        "Test Lab",
        "lab-0001",
        &user,
        &ipv4_lab_networks(
            "127.127.1.0/24",
            "172.31.1.0/24",
            "172.31.1.1",
            "172.31.1.2",
        ),
        &LabDetails::default(),
    )
    .await?;
    let image = upsert_node_image(db, NodeConfig::get_model(NodeModel::UbuntuLinux)).await?;
    create_node(db, "node1", 1, image.id.unwrap(), lab.id.unwrap()).await
}

#[tokio::test]
async fn a_new_node_is_building() -> Result<()> {
    // M1.1: `up` inserts its nodes before it creates their domains and containers.
    let db = setup_db().await?;
    let node = one_node(&db).await?;
    assert_eq!(node.state, NodeState::Building);
    Ok(())
}

#[tokio::test]
async fn a_state_write_moves_a_node_from_the_state_it_expects() -> Result<()> {
    let db = setup_db().await?;
    let id = one_node(&db).await?.id.unwrap();

    let moved =
        update_node_state_from(&db, id.clone(), NodeState::Building, NodeState::Starting).await?;
    assert_eq!(moved.map(|node| node.state), Some(NodeState::Starting));
    assert_eq!(get_node(&db, id).await?.state, NodeState::Starting);
    Ok(())
}

#[tokio::test]
async fn a_state_write_leaves_a_node_another_writer_moved() -> Result<()> {
    // M1.5 and M1.6: a writer that read the state before another writer moved the node does
    // not overwrite it.
    let db = setup_db().await?;
    let id = one_node(&db).await?.id.unwrap();
    update_node_state_from(&db, id.clone(), NodeState::Building, NodeState::Stopped).await?;

    let moved =
        update_node_state_from(&db, id.clone(), NodeState::Building, NodeState::Running).await?;
    assert!(moved.is_none());
    assert_eq!(get_node(&db, id).await?.state, NodeState::Stopped);
    Ok(())
}

#[tokio::test]
async fn a_state_write_to_a_missing_node_fails() -> Result<()> {
    let db = setup_db().await?;
    one_node(&db).await?;
    let id = RecordId::new("node", "nonexistent");
    assert!(
        update_node_state_from(&db, id, NodeState::Building, NodeState::Running)
            .await
            .is_err()
    );
    Ok(())
}
