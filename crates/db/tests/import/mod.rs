//! The SurrealDB importer, against a real SurrealDB server.
//!
//! Needs the test SurrealDB from `dev/testdb start`, signed in to as `sherpa`:
//! `SHERPA_DEV_DB_USER=sherpa SHERPA_DEV_DB_PASS=<password> SHERPA_DB_PASSWORD=<password> dev/testdb restart`
use anyhow::Result;
use jiff::Timestamp;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Ws;
use surrealdb::opt::auth::Root;

use db::{
    ImportSummary, SurrealSource, get_lab, get_node_image_by_id, get_user, import_from_surrealdb,
    list_bridges, list_links_by_lab, list_nodes_by_lab, open_in_memory,
};
use shared::data::{BridgeKind, LabState, NodeConfig, NodeModel, NodeState, RecordId};
use shared::konst::{SHERPA_DB_USER, SHERPA_PASSWORD};

const NAMESPACE: &str = "import_test";
const DATABASE: &str = "import_test";

fn source() -> SurrealSource {
    SurrealSource {
        host: "localhost".to_string(),
        port: std::env::var("SHERPA_DEV_DB_PORT")
            .unwrap_or_else(|_| "42069".to_string())
            .parse()
            .expect("SHERPA_DEV_DB_PORT must be a port number"),
        namespace: NAMESPACE.to_string(),
        database: DATABASE.to_string(),
        password: std::env::var("SHERPA_DB_PASSWORD")
            .unwrap_or_else(|_| SHERPA_PASSWORD.to_string()),
    }
}

/// A node image as a SurrealQL object, the way the old server stored it: no id, no null fields.
fn node_image_object() -> String {
    let mut value = serde_json::to_value(NodeConfig::get_model(NodeModel::AristaCeos)).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("id");
    object.retain(|_, field| !field.is_null());
    value.to_string()
}

async fn seed(source: &SurrealSource) -> Result<()> {
    let db = Surreal::new::<Ws>(format!("{}:{}/rpc", source.host, source.port)).await?;
    db.signin(Root {
        username: SHERPA_DB_USER.to_string(),
        password: source.password.clone(),
    })
    .await?;
    db.use_ns(NAMESPACE).use_db(DATABASE).await?;
    let script = format!(
        "REMOVE DATABASE IF EXISTS {DATABASE};
         DEFINE DATABASE {DATABASE};
         USE DB {DATABASE};
         CREATE user:admin CONTENT {{ username: 'admin', password_hash: 'h1', is_admin: true,
             ssh_keys: [], created_at: d'2026-01-02T03:04:05.123456789Z',
             updated_at: d'2026-01-02T03:04:05.123456789Z' }};
         CREATE user:alice CONTENT {{ username: 'alice', password_hash: 'h2', is_admin: false,
             ssh_keys: ['ssh-ed25519 AAAA alice'], created_at: d'2026-01-02T03:04:05.123456789Z',
             updated_at: d'2026-02-03T04:05:06Z' }};
         CREATE node_image:img1 CONTENT {image};
         CREATE lab:lab1 CONTENT {{ lab_id: 'test1234', name: 'lab1', user: user:alice,
             loopback_network: '127.127.1.0/24', management_network: '172.31.1.0/24',
             gateway_ipv4: '172.31.1.1', router_ipv4: '172.31.1.2', status: 'running',
             tailscale_container_id: 'abc', management_network_v6: 'fd00:1::/64' }};
         CREATE node:n1 CONTENT {{ name: 'r1', index: 1, image: node_image:img1, lab: lab:lab1,
             mgmt_ipv4: '172.31.1.11', state: 'running', cpu_count: 2, memory: 2048,
             vnc_port: 5901, cpu_pinning: [3, 4] }};
         CREATE node:n2 CONTENT {{ name: 'r2', index: 2, image: node_image:img1, lab: lab:lab1,
             state: 'running' }};
         CREATE link:k1 CONTENT {{ index: 0, kind: 'p2p', node_a: node:n1, node_b: node:n2,
             int_a: 'eth1', int_b: 'eth1', lab: lab:lab1, tap_a: 'tp1', tap_b: 'tp2',
             impairment_a_to_b: {{ delay_us: 100, jitter_us: 5, loss_percent: 1.5f,
                 reorder_percent: 0.0f, corrupt_percent: 0.0f }},
             impairment_b_to_a: {{ delay_us: 0, jitter_us: 0, loss_percent: 0.0f,
                 reorder_percent: 0.0f, corrupt_percent: 0.25f }} }};
         CREATE bridge:b1 CONTENT {{ index: 0, bridge_name: 'br0', network_name: 'net0',
             lab: lab:lab1, members: [{{ node: node:n2, interface: 'eth3' }},
                 {{ node: node:n1, interface: 'eth2' }}] }};",
        image = node_image_object()
    );
    db.query(script).await?.check()?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires the test SurrealDB from dev/testdb"]
async fn import_copies_every_record_with_its_key() -> Result<()> {
    let source = source();
    seed(&source).await?;
    let target = open_in_memory().await?;

    let summary = import_from_surrealdb(&source, &target).await?;
    assert_eq!(
        summary,
        ImportSummary {
            users: 2,
            node_images: 1,
            labs: 1,
            nodes: 2,
            links: 1,
            bridges: 1,
            bridge_members: 2,
        }
    );

    let alice = get_user(&target, "alice").await?;
    assert_eq!(alice.id, Some(RecordId::new("user", "alice")));
    assert_eq!(alice.ssh_keys, vec!["ssh-ed25519 AAAA alice".to_string()]);
    assert_eq!(
        alice.created_at,
        "2026-01-02T03:04:05.123456789Z".parse::<Timestamp>()?
    );
    assert!(get_user(&target, "admin").await?.is_admin);

    let image = get_node_image_by_id(&target, RecordId::new("node_image", "img1"))
        .await?
        .expect("imported node image");
    assert_eq!(image.model, NodeModel::AristaCeos);

    let lab = get_lab(&target, "test1234").await?;
    let lab_id = RecordId::new("lab", "lab1");
    assert_eq!(lab.id, Some(lab_id.clone()));
    assert_eq!(lab.user, RecordId::new("user", "alice"));
    assert_eq!(lab.status, LabState::Running);
    assert_eq!(lab.tailscale_container_id.as_deref(), Some("abc"));
    assert_eq!(lab.management_network_v6.as_deref(), Some("fd00:1::/64"));

    let nodes = list_nodes_by_lab(&target, lab_id.clone()).await?;
    let r1 = nodes
        .iter()
        .find(|node| node.name == "r1")
        .expect("node r1");
    assert_eq!(r1.id, Some(RecordId::new("node", "n1")));
    assert_eq!(r1.state, NodeState::Running);
    assert_eq!(r1.vnc_port, Some(5901));
    assert_eq!(r1.cpu_pinning, Some(vec![3, 4]));

    let links = list_links_by_lab(&target, lab_id.clone()).await?;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].id, Some(RecordId::new("link", "k1")));
    assert_eq!(links[0].kind, BridgeKind::P2p);
    assert_eq!(links[0].impairment_a_to_b.delay_us, 100);
    assert_eq!(links[0].impairment_a_to_b.loss_percent, 1.5);
    assert_eq!(links[0].impairment_b_to_a.corrupt_percent, 0.25);

    let bridges = list_bridges(&target, &lab_id).await?;
    assert_eq!(bridges.len(), 1);
    let members: Vec<(RecordId, String)> = bridges[0]
        .members
        .iter()
        .map(|member| (member.node.clone(), member.interface.clone()))
        .collect();
    assert_eq!(
        members,
        vec![
            (RecordId::new("node", "n2"), "eth3".to_string()),
            (RecordId::new("node", "n1"), "eth2".to_string()),
        ]
    );

    // A second import never mixes with existing data, and changes nothing.
    let error = import_from_surrealdb(&source, &target).await.unwrap_err();
    assert!(format!("{error:#}").contains("empty database"), "{error:#}");
    assert_eq!(list_nodes_by_lab(&target, lab_id).await?.len(), 2);
    Ok(())
}
