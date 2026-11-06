use anyhow::Result;
use db::{
    Database, LabNetworks, apply_schema, create_node, find_node_image_by_model_kind_version,
    open_in_memory,
};
use shared::data::{BridgeKind, DbLab, DbLink, DbNode, LinkImpairment, NodeConfig, NodeModel};

/// Open a fresh in-memory database with the schema applied.
///
/// Each test gets its own database, so tests are isolated and need no cleanup.
///
/// # Errors
///
/// Fails when the database cannot be opened or the schema applied.
pub async fn setup_db() -> Result<Database> {
    let db = open_in_memory().await?;
    apply_schema(&db).await?;
    Ok(db)
}

/// Helper to create a test config
#[must_use]
pub fn create_test_config(model: NodeModel) -> NodeConfig {
    NodeConfig::get_model(model)
}

/// Test helper to create a node with a specific model
///
/// This is a convenience wrapper for tests that handles the config lookup automatically.
/// Equivalent to the old `create_lab_node()` but for test use only.
///
/// # Errors
///
/// Fails when the node cannot be created.
///
/// # Panics
///
/// Panics when the model has no image, or the image or the lab has no id.
pub async fn create_test_node_with_model(
    db: &Database,
    name: &str,
    index: u16,
    model: NodeModel,
    lab: &DbLab,
) -> Result<DbNode> {
    // Get the model kind from the NodeModel
    let config_template = NodeConfig::get_model(model);
    let kind = config_template.kind;
    let version = config_template.version;

    let config = find_node_image_by_model_kind_version(db, &model, &kind, &version)
        .await?
        .expect("Config should exist for this model");
    let config_id = config.id.clone().expect("Config should have an id");
    let lab_id = lab.id.clone().expect("Lab should have an id");

    create_node(db, name, index, config_id, lab_id).await
}

/// IPv4-only lab networks for tests that do not exercise IPv6 allocation.
#[must_use]
pub fn ipv4_lab_networks(
    loopback_network: &str,
    management_network: &str,
    gateway_ipv4: &str,
    router_ipv4: &str,
) -> LabNetworks {
    LabNetworks {
        loopback_network: loopback_network.to_string(),
        management_network: management_network.to_string(),
        gateway_ipv4: gateway_ipv4.to_string(),
        router_ipv4: router_ipv4.to_string(),
        management_network_v6: None,
        loopback_network_v6: None,
        gateway_ipv6: None,
        router_ipv6: None,
    }
}

/// A link of `lab`, numbered `index`, from `a`'s `int_a` to `b`'s `int_b`, unimpaired.
///
/// # Panics
///
/// Panics when the lab or a node has no id.
#[must_use]
pub fn test_link(
    lab: &DbLab,
    index: u16,
    kind: BridgeKind,
    a: &DbNode,
    int_a: &str,
    b: &DbNode,
    int_b: &str,
) -> DbLink {
    DbLink {
        id: None,
        index,
        kind,
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
