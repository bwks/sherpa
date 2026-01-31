use anyhow::{Context, Result, anyhow};

use data::{DbLab, DbLink, DbNode, LabLinkData, NodeModel};

use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::helpers::{get_config_id, get_lab_id, get_node_id};
use crate::node_config::get_node_config;
use crate::node::create_node;

/// Assign a lab node
///
/// This is a convenience wrapper around `create_node()` that accepts a `NodeModel`
/// enum and automatically looks up the corresponding config. It also accepts a `DbLab`
/// reference for convenience.
///
/// For more flexible node creation, use `create_node()` directly from the `node` module.
pub async fn create_lab_node(
    db: &Surreal<Client>,
    name: &str,
    index: u16,
    model: NodeModel,
    lab: &DbLab,
) -> Result<DbNode> {
    let config = get_node_config(db, &model).await?;
    let config_id = get_config_id(&config)?;
    let lab_id = get_lab_id(lab)?;

    create_node(db, name, index, config_id, lab_id).await
}

/// Create a link record between two nodes
pub async fn create_lab_link(
    db: &Surreal<Client>,
    lab: &DbLab,
    link_data: &LabLinkData,
) -> Result<DbLink> {
    let node_a_id = get_node_id(&link_data.node_a)?;
    let node_b_id = get_node_id(&link_data.node_b)?;
    let lab_id = get_lab_id(lab)?;

    let link: Option<DbLink> = db
        .create("link")
        .content(DbLink {
            id: None,
            index: link_data.index,
            kind: link_data.kind.to_owned(),
            node_a: node_a_id,
            node_b: node_b_id,
            int_a: link_data.int_a.to_owned(),
            int_b: link_data.int_b.to_owned(),
            lab: lab_id,
            bridge_a: link_data.bridge_a.to_owned(),
            bridge_b: link_data.bridge_b.to_owned(),
            veth_a: link_data.veth_a.to_owned(),
            veth_b: link_data.veth_b.to_owned(),
        })
        .await
        .context(format!(
            "Error creating link:\n index: {}\n node_a: {}\n node_b: {}\n int_a: {}\n int_b: {}\n",
            link_data.index,
            link_data.node_a.name,
            link_data.node_b.name,
            link_data.int_a,
            link_data.int_b
        ))?;

    link.ok_or_else(|| {
        anyhow!(
            "Link was not created:\n index: {}\n node_a: {}\n node_b: {}\n int_a: {}\n int_b: {}\n lab: {}\n",
            link_data.index,
            link_data.node_a.name,
            link_data.node_b.name,
            link_data.int_a,
            link_data.int_b,
            lab.name,
        )
    })
}
