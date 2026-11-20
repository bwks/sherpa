use anyhow::Result;

use db::{
    Database, DefaultRequest, LabDetails, count_nodes_by_lab, create_bridge, create_lab,
    create_link, create_node, create_user, delete_lab_from, delete_user,
    find_node_image_by_model_kind_version, get_lab, get_user, list_bridges, list_links_by_lab,
    list_nodes_by_lab, upsert_node_image,
};
use shared::data::{
    BridgeKind, BridgeMember, DbLab, DbLink, DbNode, LabPhase, LinkImpairment, ModelKind,
    NodeModel, RecordId,
};

use crate::{create_test_node_with_model, ipv4_lab_networks, setup_db};

// ============================================================================
// Cascade deletes — lab
// ============================================================================

// ============================================================================
// User deletes are rejected while the user owns labs
// ============================================================================

/// Deleting a user who owns a lab is rejected by the schema, and the lab and its nodes
/// remain. A cascade would delete the records but leave the lab's VMs, containers and
/// networks running with nothing left to manage them.
#[tokio::test]
async fn test_delete_user_is_rejected_while_they_own_labs() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(
        &db,
        "doomed_user".to_string(),
        "TestPass123!",
        false,
        vec![],
    )
    .await?;
    let user_id = user.id.clone().expect("user has id");
    upsert_node_image(
        &db,
        ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
        DefaultRequest::Keep,
    )
    .await?;

    let lab = create_lab(
        &db,
        "Doomed Lab",
        "doom0001",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let lab_rid = lab.id.clone().expect("lab has id");

    create_test_node_with_model(&db, "node1", 1, NodeModel::UbuntuLinux, &lab).await?;
    create_test_node_with_model(&db, "node2", 2, NodeModel::UbuntuLinux, &lab).await?;

    // Verify setup
    assert_eq!(count_nodes_by_lab(&db, lab_rid.clone()).await?, 2);

    // Deleting the user must fail while the lab exists
    assert!(delete_user(&db, user_id.clone()).await.is_err());
    assert!(get_lab(&db, "doom0001").await.is_ok());
    assert_eq!(count_nodes_by_lab(&db, lab_rid).await?, 2);

    // Once the lab is gone, the user can be deleted
    assert!(delete_lab_from(&db, "doom0001", LabPhase::Creating).await?);
    delete_user(&db, user_id).await?;

    Ok(())
}

// ============================================================================
// Cross-table query isolation
// ============================================================================

/// Nodes from one lab don't appear when querying another lab.
#[tokio::test]
async fn test_list_nodes_by_lab_isolation() -> Result<()> {
    let db = setup_db().await?;

    let user = create_user(&db, "iso_user".to_string(), "TestPass123!", false, vec![]).await?;
    upsert_node_image(
        &db,
        ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
        DefaultRequest::Keep,
    )
    .await?;

    let lab_a = create_lab(
        &db,
        "Lab A",
        "laba0001",
        &user,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    let lab_b = create_lab(
        &db,
        "Lab B",
        "labb0001",
        &user,
        &ipv4_lab_networks("172.31.2.0/24", "172.31.2.1", "172.31.2.2"),
        &LabDetails::default(),
    )
    .await?;

    create_test_node_with_model(&db, "a-node1", 1, NodeModel::UbuntuLinux, &lab_a).await?;
    create_test_node_with_model(&db, "a-node2", 2, NodeModel::UbuntuLinux, &lab_a).await?;
    create_test_node_with_model(&db, "b-node1", 1, NodeModel::UbuntuLinux, &lab_b).await?;

    let nodes_a = list_nodes_by_lab(&db, lab_a.id.clone().unwrap()).await?;
    let nodes_b = list_nodes_by_lab(&db, lab_b.id.clone().unwrap()).await?;

    assert_eq!(nodes_a.len(), 2);
    assert_eq!(nodes_b.len(), 1);

    let names_a: Vec<&str> = nodes_a.iter().map(|n| n.name.as_str()).collect();
    assert!(names_a.contains(&"a-node1"));
    assert!(names_a.contains(&"a-node2"));
    assert_eq!(nodes_b[0].name, "b-node1");

    Ok(())
}

// ============================================================================
// Labs by user isolation
// ============================================================================

/// Each user only sees their own labs.
#[tokio::test]
async fn test_list_labs_by_user_isolation() -> Result<()> {
    let db = setup_db().await?;

    let user_a = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![]).await?;
    let user_b = create_user(&db, "bob".to_string(), "TestPass123!", false, vec![]).await?;

    create_lab(
        &db,
        "Alice Lab 1",
        "alab0001",
        &user_a,
        &ipv4_lab_networks("172.31.1.0/24", "172.31.1.1", "172.31.1.2"),
        &LabDetails::default(),
    )
    .await?;
    create_lab(
        &db,
        "Alice Lab 2",
        "alab0002",
        &user_a,
        &ipv4_lab_networks("172.31.2.0/24", "172.31.2.1", "172.31.2.2"),
        &LabDetails::default(),
    )
    .await?;
    create_lab(
        &db,
        "Bob Lab 1",
        "blab0001",
        &user_b,
        &ipv4_lab_networks("172.31.3.0/24", "172.31.3.1", "172.31.3.2"),
        &LabDetails::default(),
    )
    .await?;

    let alice_labs = db::list_labs_by_user(&db, user_a.id.clone().unwrap()).await?;
    let bob_labs = db::list_labs_by_user(&db, user_b.id.clone().unwrap()).await?;

    assert_eq!(alice_labs.len(), 2);
    assert_eq!(bob_labs.len(), 1);
    assert_eq!(bob_labs[0].name, "Bob Lab 1");

    Ok(())
}

// ============================================================================
// Foreign keys, transactions and concurrency
// ============================================================================

/// A user, the Ubuntu image and a lab, for the tests below.
async fn lab_fixture(db: &Database, lab_id: &str, octet: u8) -> Result<DbLab> {
    let user = create_user(db, format!("owner{octet}"), "TestPass123!", false, vec![]).await?;
    upsert_node_image(
        db,
        ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
        DefaultRequest::Keep,
    )
    .await?;
    create_lab(
        db,
        &format!("Lab {lab_id}"),
        lab_id,
        &user,
        &ipv4_lab_networks(
            &format!("172.31.{octet}.0/24"),
            &format!("172.31.{octet}.1"),
            &format!("172.31.{octet}.2"),
        ),
        &LabDetails::default(),
    )
    .await
}

/// Deleting a lab removes its nodes, links, bridges and bridge members, and leaves other labs.
#[tokio::test]
async fn test_delete_lab_cascades_through_every_level() -> Result<()> {
    let db = setup_db().await?;
    let lab = lab_fixture(&db, "fkey0002", 11).await?;
    let other = create_lab(
        &db,
        "Other",
        "fkey0003",
        &get_user(&db, "owner11").await?,
        &ipv4_lab_networks("172.31.12.0/24", "172.31.12.1", "172.31.12.2"),
        &LabDetails::default(),
    )
    .await?;
    let lab_rid = lab.id.clone().expect("lab has id");
    let other_rid = other.id.clone().expect("other lab has id");

    let node1 = create_test_node_with_model(&db, "n1", 1, NodeModel::UbuntuLinux, &lab).await?;
    let node2 = create_test_node_with_model(&db, "n2", 2, NodeModel::UbuntuLinux, &lab).await?;
    let kept = create_test_node_with_model(&db, "n1", 1, NodeModel::UbuntuLinux, &other).await?;
    let n1 = node1.id.clone().expect("n1 has id");
    let n2 = node2.id.clone().expect("n2 has id");
    create_link(
        &db,
        DbLink {
            id: None,
            index: 0,
            kind: BridgeKind::default(),
            node_a: n1.clone(),
            node_b: n2.clone(),
            int_a: "eth1".to_string(),
            int_b: "eth1".to_string(),
            lab: lab_rid.clone(),
            tap_a: String::new(),
            tap_b: String::new(),
            impairment_a_to_b: LinkImpairment::default(),
            impairment_b_to_a: LinkImpairment::default(),
        },
    )
    .await?;
    create_bridge(
        &db,
        0,
        "brs0-fkey0002".to_string(),
        "sherpa-bridge0-fkey0002".to_string(),
        lab_rid.clone(),
        vec![BridgeMember {
            node: n2,
            interface: "eth2".to_string(),
        }],
    )
    .await?;

    assert!(delete_lab_from(&db, &lab.lab_id, LabPhase::Creating).await?);

    assert!(get_lab(&db, "fkey0002").await.is_err());
    assert_eq!(count_nodes_by_lab(&db, lab_rid.clone()).await?, 0);
    assert_eq!(list_links_by_lab(&db, lab_rid.clone()).await?.len(), 0);
    assert!(list_bridges(&db, &lab_rid).await?.is_empty());
    let other_nodes = list_nodes_by_lab(&db, other_rid).await?;
    assert_eq!(other_nodes.len(), 1);
    assert_eq!(other_nodes[0].id, kept.id);
    Ok(())
}

/// A bridge whose member refers to a node that does not exist is not stored at all: the bridge
/// row and its members are written in one transaction.
#[tokio::test]
async fn private_bridge_008_a_bridge_is_recorded_with_its_members_or_not_and_goes_with_its_lab()
-> Result<()> {
    // A bridge whose member refers to a node that does not exist is not stored at all: the bridge
    // row and its members are written in one transaction.
    let db = setup_db().await?;
    let lab = lab_fixture(&db, "txn00001", 14).await?;
    let lab_rid = lab.id.clone().expect("lab has id");
    let node = create_test_node_with_model(&db, "n1", 1, NodeModel::UbuntuLinux, &lab).await?;

    let result = create_bridge(
        &db,
        0,
        "brs0-txn00001".to_string(),
        "sherpa-bridge0-txn00001".to_string(),
        lab_rid.clone(),
        vec![
            BridgeMember {
                node: node.id.clone().expect("node has id"),
                interface: "eth1".to_string(),
            },
            BridgeMember {
                node: RecordId::new("node", "doesnotexist"),
                interface: "eth1".to_string(),
            },
        ],
    )
    .await;

    assert!(result.is_err(), "a member with no node must be refused");
    assert!(list_bridges(&db, &lab_rid).await?.is_empty());

    // The same bridge with only the real member is stored, so the failure came from the member.
    create_bridge(
        &db,
        0,
        "brs0-txn00001".to_string(),
        "sherpa-bridge0-txn00001".to_string(),
        lab_rid.clone(),
        vec![BridgeMember {
            node: node.id.clone().expect("node has id"),
            interface: "eth1".to_string(),
        }],
    )
    .await?;
    assert_eq!(list_bridges(&db, &lab_rid).await?.len(), 1);

    // The server's destroy deletes links, then nodes, then the lab. A bridge must not stop
    // its members' nodes from being deleted, and it keeps which interface of each node it holds.
    let db = setup_db().await?;

    let user = create_user(
        &db,
        "bridge_user".to_string(),
        "TestPass123!",
        false,
        vec![],
    )
    .await?;
    upsert_node_image(
        &db,
        ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
        DefaultRequest::Keep,
    )
    .await?;

    let lab = create_lab(
        &db,
        "Bridge Lab",
        "brdg0001",
        &user,
        &ipv4_lab_networks("172.31.2.0/24", "172.31.2.1", "172.31.2.2"),
        &LabDetails::default(),
    )
    .await?;
    let lab_rid = lab.id.clone().expect("lab has id");

    let node1 = create_test_node_with_model(&db, "n1", 1, NodeModel::UbuntuLinux, &lab).await?;
    let node2 = create_test_node_with_model(&db, "n2", 2, NodeModel::UbuntuLinux, &lab).await?;
    let members = vec![
        BridgeMember {
            node: node1.id.clone().expect("n1 has id"),
            interface: "eth1".to_string(),
        },
        BridgeMember {
            node: node2.id.clone().expect("n2 has id"),
            interface: "eth3".to_string(),
        },
    ];
    create_bridge(
        &db,
        0,
        "brs0-brdg0001".to_string(),
        "sherpa-bridge0-private-brdg0001".to_string(),
        lab_rid.clone(),
        members.clone(),
    )
    .await?;

    let bridges = list_bridges(&db, &lab_rid).await?;
    assert_eq!(bridges.len(), 1);
    assert_eq!(bridges[0].members, members);

    assert!(delete_lab_from(&db, &lab.lab_id, LabPhase::Creating).await?);

    assert_eq!(count_nodes_by_lab(&db, lab_rid.clone()).await?, 0);
    assert!(list_bridges(&db, &lab_rid).await?.is_empty());
    Ok(())
}

/// Many tasks writing through one shared handle all succeed: writes are queued, not refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_concurrent_writes_through_one_database_all_succeed() -> Result<()> {
    const TASKS: u16 = 32;
    let db = setup_db().await?;
    let lab = lab_fixture(&db, "conc0001", 15).await?;
    let lab_rid = lab.id.clone().expect("lab has id");
    let template = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
    let image_id = find_node_image_by_model_kind_version(
        &db,
        &template.model,
        &template.kind(),
        &template.version,
    )
    .await?
    .expect("image exists")
    .id
    .expect("image has id");

    let mut handles = Vec::new();
    for index in 0..TASKS {
        let db = db.clone();
        let image_id = image_id.clone();
        let lab_rid = lab_rid.clone();
        handles.push(tokio::spawn(async move {
            create_node(&db, &format!("node{index}"), index, image_id, lab_rid).await
        }));
    }
    for handle in handles {
        handle.await??;
    }

    assert_eq!(count_nodes_by_lab(&db, lab_rid).await?, usize::from(TASKS));
    Ok(())
}

// ============================================================================
// Each node interface is used once
// ============================================================================

/// A P2p link from `a`'s `int_a` to `b`'s `int_b`, numbered `index`.
fn p2p_link(lab: &DbLab, index: u16, a: &DbNode, int_a: &str, b: &DbNode, int_b: &str) -> DbLink {
    DbLink {
        id: None,
        index,
        kind: BridgeKind::P2p,
        node_a: a.id.clone().expect("node has id"),
        node_b: b.id.clone().expect("node has id"),
        int_a: int_a.to_string(),
        int_b: int_b.to_string(),
        lab: lab.id.clone().expect("lab has id"),
        tap_a: String::new(),
        tap_b: String::new(),
        impairment_a_to_b: LinkImpairment::default(),
        impairment_b_to_a: LinkImpairment::default(),
    }
}

#[tokio::test]
async fn private_bridge_009_an_interface_is_recorded_in_one_link_or_one_bridge_only() -> Result<()>
{
    let db = setup_db().await?;
    let lab = lab_fixture(&db, "once0001", 21).await?;
    let n1 = create_test_node_with_model(&db, "n1", 1, NodeModel::UbuntuLinux, &lab).await?;
    let n2 = create_test_node_with_model(&db, "n2", 2, NodeModel::UbuntuLinux, &lab).await?;
    let n3 = create_test_node_with_model(&db, "n3", 3, NodeModel::UbuntuLinux, &lab).await?;
    let first = create_link(&db, p2p_link(&lab, 0, &n1, "eth1", &n2, "eth1")).await?;

    let error = create_link(&db, p2p_link(&lab, 5, &n1, "eth1", &n2, "eth1"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Failed to create link"), "{error}");
    for (index, link) in [
        (1, p2p_link(&lab, 1, &n1, "eth1", &n3, "eth1")),
        (2, p2p_link(&lab, 2, &n3, "eth2", &n2, "eth1")),
        (3, p2p_link(&lab, 3, &n2, "eth1", &n1, "eth1")),
        (4, p2p_link(&lab, 4, &n3, "eth3", &n3, "eth3")),
    ] {
        assert!(create_link(&db, link).await.is_err(), "link {index}");
    }
    // Other interfaces of the same two nodes make a link of their own.
    let second = create_link(&db, p2p_link(&lab, 6, &n1, "eth2", &n2, "eth2")).await?;
    assert_ne!(first.id, second.id);
    assert_eq!(
        (&first.node_a, &first.node_b),
        (&second.node_a, &second.node_b)
    );
    assert_eq!(
        list_links_by_lab(&db, lab.id.clone().expect("lab has id"))
            .await?
            .len(),
        2
    );

    let db = setup_db().await?;
    let lab = lab_fixture(&db, "once0002", 22).await?;
    let lab_rid = lab.id.clone().expect("lab has id");
    let n1 = create_test_node_with_model(&db, "n1", 1, NodeModel::UbuntuLinux, &lab).await?;
    let n2 = create_test_node_with_model(&db, "n2", 2, NodeModel::UbuntuLinux, &lab).await?;
    create_link(&db, p2p_link(&lab, 0, &n1, "eth1", &n2, "eth1")).await?;

    let member = |node: &DbNode, interface: &str| BridgeMember {
        node: node.id.clone().expect("node has id"),
        interface: interface.to_string(),
    };
    let bridged = create_bridge(
        &db,
        0,
        "brs0-once0002".to_string(),
        "sherpa-bridge0-once0002".to_string(),
        lab_rid.clone(),
        vec![member(&n1, "eth1")],
    )
    .await;
    assert!(bridged.is_err(), "a linked interface joined a bridge");

    create_bridge(
        &db,
        1,
        "brs1-once0002".to_string(),
        "sherpa-bridge1-once0002".to_string(),
        lab_rid.clone(),
        vec![member(&n1, "eth2"), member(&n2, "eth2")],
    )
    .await?;
    assert!(
        create_link(&db, p2p_link(&lab, 1, &n1, "eth2", &n2, "eth3"))
            .await
            .is_err()
    );
    let twice = create_bridge(
        &db,
        2,
        "brs2-once0002".to_string(),
        "sherpa-bridge2-once0002".to_string(),
        lab_rid,
        vec![member(&n1, "eth3"), member(&n1, "eth3")],
    )
    .await;
    assert!(twice.is_err(), "an interface joined one bridge twice");
    Ok(())
}
