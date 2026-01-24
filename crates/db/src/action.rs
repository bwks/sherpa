use anyhow::{Context, Result, anyhow};

use super::model::{DbLab, DbLink, DbNode, DbUser, NodeVariant};
use data::NodeModel;

use surrealdb::RecordId;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// Get a user form the database
pub async fn get_user(db: &Surreal<Client>, username: &str) -> Result<DbUser> {
    let user: Option<DbUser> = db
        .select(("user", username))
        .await
        .context(format!("Failed querying user from database: {username}"))?;
    dbg!(&user);
    user.ok_or_else(|| anyhow!("User not found: {username}"))
}

/// Get a user's id from a user record.
async fn get_user_id(user: &DbUser) -> Result<RecordId> {
    user.id
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("User record has no id field:\n '{:#?}'", user))
}

/// Create a lab record.
pub async fn create_lab(
    db: &Surreal<Client>,
    name: &str,
    lab_id: &str,
    user: &DbUser,
) -> Result<DbLab> {
    let user_id = get_user_id(user).await?;

    let lab: Option<DbLab> = db
        .create("lab")
        .content(DbLab {
            id: None,
            name: name.to_string(),
            lab_id: lab_id.to_string(),
            user: user_id,
        })
        .await
        .context("Error creating lab:\n name: `{name}`\n lab_id: {lab_id}\n")?;

    dbg!(&lab);

    lab.ok_or_else(|| anyhow!("Lab was not created:\n name: `{name}`\n lab_id: {lab_id}\n"))
}

/// Get a lab's id from a lab record.
fn get_lab_id(lab: &DbLab) -> Result<RecordId> {
    lab.id
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("Lab has no id field:\n {:#?}", lab))
}

/// Get a variant's id from a variant record.
fn get_variant_id(variant: &NodeVariant) -> Result<RecordId> {
    variant
        .id
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("variant has no id field:\n {:#?}", variant))
}

/// Get a node's id from a node record.
fn get_node_id(node: &DbNode) -> Result<RecordId> {
    node.id
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("Node has no id field:\n {:#?}", node))
}

/// Delete a lab
pub async fn delete_lab(db: &Surreal<Client>, lab_id: &str) -> Result<()> {
    let mut response = db
        .query("SELECT * FROM ONLY lab WHERE lab_id = $lab_id")
        .bind(("lab_id", lab_id.to_string()))
        .await
        .context(format!("Failed to query lab_id from database: {lab_id}"))?;

    let db_lab: Option<DbLab> = response.take(0)?;

    let lab = db_lab.ok_or_else(|| anyhow!("Lab with lab_id not found: {lab_id}"))?;

    let lab_record_id = get_lab_id(&lab)?;

    let _deleted: Option<DbLab> = db
        .delete(lab_record_id)
        .await
        .context(format!("Failed to delete lab: {lab_id}"))?;

    Ok(())
}

/// Delete all nodes for a lab
pub async fn delete_lab_nodes(db: &Surreal<Client>, lab_id: &str) -> Result<()> {
    let mut response = db
        .query("SELECT * FROM ONLY lab WHERE lab_id = $lab_id")
        .bind(("lab_id", lab_id.to_string()))
        .await
        .context(format!("Failed to query lab from database: {lab_id}"))?;

    let db_lab: Option<DbLab> = response.take(0)?;

    let lab = db_lab.ok_or_else(|| anyhow!("Lab with lab_id not found: {lab_id}"))?;

    let lab_record_id = get_lab_id(&lab)?;

    let _deleted: Vec<DbNode> = db
        .query("DELETE node WHERE lab = $lab_id")
        .bind(("lab_id", lab_record_id))
        .await
        .context(format!("Failed to delete nodes for lab: {lab_id}"))?
        .take(0)?;

    Ok(())
}

/// Get node_variant from node_model
async fn get_node_variant(db: &Surreal<Client>, node_model: &NodeModel) -> Result<NodeVariant> {
    let model_id = node_model.to_string();

    let mut response = db
        .query("SELECT * FROM ONLY node_variant WHERE model = type::thing('node_model', $model_id)")
        .bind(("model_id", model_id))
        .await
        .context(format!(
            "Failed to query node_variant from database: {node_model}"
        ))?;

    let variant: Option<NodeVariant> = response.take(0)?;

    dbg!("{}", &variant);
    variant.ok_or_else(|| anyhow!("Node variant not found for model: {node_model}"))
}

/// Assign a lab node
pub async fn create_lab_node(
    db: &Surreal<Client>,
    name: &str,
    index: u16,
    model: NodeModel,
    lab: &DbLab,
) -> Result<DbNode> {
    let variant = get_node_variant(&db, &model).await?;
    let variant_id = get_variant_id(&variant)?;
    let lab_id = get_lab_id(lab)?;

    let node: Option<DbNode> = db
        .create("node")
        .content(DbNode {
            id: None,
            name: name.to_string(),
            variant: variant_id,
            index,
            lab: lab_id,
        })
        .await
        .context("Error creating node:\n name: `{name}`\n lab_id: {lab_id}\n")?;

    dbg!(&node);

    node.ok_or_else(|| {
        anyhow!(
            "Node was not created:\n node name: `{}`\n lab name: {}\n lab id: {}\n",
            name,
            lab.name,
            lab.lab_id,
        )
    })
}

/// Create a link record between two nodes
pub async fn create_lab_link(
    db: &Surreal<Client>,
    link_id: u16,
    node_a: &DbNode,
    node_b: &DbNode,
    int_a: &str,
    int_b: &str,
    lab: &DbLab,
) -> Result<DbLink> {
    let node_a_id = get_node_id(node_a)?;
    let node_b_id = get_node_id(node_b)?;
    let lab_id = get_lab_id(lab)?;

    let link: Option<DbLink> = db
        .create("link")
        .content(DbLink {
            id: None,
            link_id,
            node_a: node_a_id,
            node_b: node_b_id,
            int_a: int_a.to_string(),
            int_b: int_b.to_string(),
            lab: lab_id,
        })
        .await
        .context(format!(
            "Error creating link:\n link_id: {}\n node_a: {}\n node_b: {}\n int_a: {}\n int_b: {}\n",
            link_id, node_a.name, node_b.name, int_a, int_b
        ))?;

    dbg!(&link);

    link.ok_or_else(|| {
        anyhow!(
            "Link was not created:\n link_id: {}\n node_a: {}\n node_b: {}\n int_a: {}\n int_b: {}\n lab: {}\n",
            link_id,
            node_a.name,
            node_b.name,
            int_a,
            int_b,
            lab.name,
        )
    })
}

/// Delete all links for a lab
pub async fn delete_lab_links(db: &Surreal<Client>, lab_id: &str) -> Result<()> {
    let mut response = db
        .query("SELECT * FROM ONLY lab WHERE lab_id = $lab_id")
        .bind(("lab_id", lab_id.to_string()))
        .await
        .context(format!("Failed to query lab from database: {lab_id}"))?;

    let db_lab: Option<DbLab> = response.take(0)?;

    let lab = db_lab.ok_or_else(|| anyhow!("Lab with lab_id not found: {lab_id}"))?;

    let lab_record_id = get_lab_id(&lab)?;

    let _deleted: Vec<DbLink> = db
        .query("DELETE link WHERE lab = $lab_id")
        .bind(("lab_id", lab_record_id))
        .await
        .context(format!("Failed to delete links for lab: {lab_id}"))?
        .take(0)?;

    Ok(())
}
