use anyhow::{Context, Result, anyhow};

use data::{DbLab, DbLink, DbNode, LabLinkData, NodeModel};

use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::helpers::{get_config_id, get_lab_id, get_node_id};
use crate::node_config::get_node_config;

/// Assign a lab node
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

    let node: Option<DbNode> = db
        .create("node")
        .content(DbNode {
            id: None,
            name: name.to_string(),
            config: config_id,
            index,
            lab: lab_id.clone(),
        })
        .await
        .context(format!(
            "Error creating node:\n name: `{name}`\n lab_id: {lab_id}\n"
        ))?;

    node.ok_or_else(|| {
        anyhow!(
            "Node was not created:\n node name: `{}`\n lab name: {}\n lab id: {:?}\n",
            name,
            lab.name,
            lab.id,
        )
    })
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
