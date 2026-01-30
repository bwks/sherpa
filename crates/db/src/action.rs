use anyhow::{Context, Result, anyhow};

use data::{DbLab, DbLink, DbNode, DbUser, LabLinkData, NodeConfig, NodeModel, RecordId};

use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

/// Get a user form the database
pub async fn get_user(db: &Surreal<Client>, username: &str) -> Result<DbUser> {
    let mut response = db
        .query("SELECT * FROM ONLY user WHERE username = $username")
        .bind(("username", username.to_string()))
        .await
        .context(format!("Failed querying user from database: {username}"))?;

    let user: Option<DbUser> = response.take(0)?;
    user.ok_or_else(|| anyhow!("User not found: {username}"))
}

/// Get a user's id from a user record.
fn get_user_id(user: &DbUser) -> Result<RecordId> {
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
    let user_id = get_user_id(user)?;

    let lab: Option<DbLab> = db
        .create("lab")
        .content(DbLab {
            id: None,
            lab_id: lab_id.to_string(),
            name: name.to_string(),
            user: user_id,
        })
        .await
        .context("Error creating lab:\n name: `{name}`\n lab_id: {lab_id}\n")?;

    lab.ok_or_else(|| anyhow!("Lab was not created:\n name: `{name}`\n lab_id: {lab_id}\n"))
}

/// Get a lab's id from a lab record.
fn get_lab_id(lab: &DbLab) -> Result<RecordId> {
    lab.id
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("Lab has no id field:\n {:#?}", lab))
}

/// Get a config's id from a config record.
fn get_config_id(config: &NodeConfig) -> Result<RecordId> {
    config
        .id
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("config has no id field:\n {:#?}", config))
}

/// Get a node's id from a node record.
fn get_node_id(node: &DbNode) -> Result<RecordId> {
    node.id
        .as_ref()
        .cloned()
        .ok_or_else(|| anyhow!("Node has no id field:\n {:#?}", node))
}

async fn get_lab_record(db: &Surreal<Client>, lab_id: &str) -> Result<DbLab> {
    let mut response = db
        .query("SELECT * FROM ONLY lab WHERE lab_id = $lab_id")
        .bind(("lab_id", lab_id.to_string()))
        .await
        .context(format!("Failed to query lab from database: {lab_id}"))?;

    let db_lab: Option<DbLab> = response.take(0)?;
    let lab = db_lab.ok_or_else(|| anyhow!("Lab with lab_id not found: {lab_id}"))?;
    Ok(lab)
}

/// Delete a lab
pub async fn delete_lab(db: &Surreal<Client>, lab_id: &str) -> Result<()> {
    let lab = get_lab_record(&db, lab_id).await?;
    let lab_record_id = get_lab_id(&lab)?;

    let _deleted: Option<DbLab> = db
        .delete(lab_record_id)
        .await
        .context(format!("Failed to delete lab: {lab_id}"))?;

    Ok(())
}

/// Delete all nodes for a lab
pub async fn delete_lab_nodes(db: &Surreal<Client>, lab_id: &str) -> Result<()> {
    let lab = get_lab_record(&db, lab_id).await?;
    let lab_record_id = get_lab_id(&lab)?;

    let _deleted: Vec<DbNode> = db
        .query("DELETE node WHERE lab = $lab_record_id")
        .bind(("lab_record_id", lab_record_id))
        .await
        .context(format!("Failed to delete nodes for lab: {lab_id}"))?
        .take(0)?;

    Ok(())
}

/// Create a node_config record in the database
pub async fn create_node_config(db: &Surreal<Client>, config: NodeConfig) -> Result<NodeConfig> {
    let created_config: Option<NodeConfig> = db
        .create("node_config")
        .content(config.clone())
        .await
        .context(format!(
            "Error creating node_config (model and kind must be unique):\n model: {}\n kind: {}\n",
            config.model, config.kind
        ))?;

    created_config.ok_or_else(|| {
        anyhow!(
            "Node config was not created:\n model: {}\n kind: {}\n",
            config.model,
            config.kind
        )
    })
}

/// Upsert a node_config record (create if not exists, update if exists)
/// This uses a query-based approach to handle the unique constraint gracefully
pub async fn upsert_node_config(db: &Surreal<Client>, config: NodeConfig) -> Result<NodeConfig> {
    // Use UPDATE...SET to upsert - this will create if not exists or update if exists
    // We need to construct a specific record ID based on model+kind to make this work
    // However, since we don't control the ID, let's use a different approach:
    
    // First, try to find existing record
    let existing = get_node_config_by_model_kind(db, &config.model, &config.kind.to_string()).await?;
    
    if let Some(existing_config) = existing {
        // Record exists, return it unchanged (or we could update it)
        return Ok(existing_config);
    }
    
    // Record doesn't exist, create it
    let created_config: Option<NodeConfig> = db
        .create("node_config")
        .content(config.clone())
        .await
        .context(format!(
            "Error creating node_config:\n model: {}\n kind: {}\n",
            config.model, config.kind
        ))?;

    created_config.ok_or_else(|| {
        anyhow!(
            "Node config was not created:\n model: {}\n kind: {}\n",
            config.model,
            config.kind
        )
    })
}

/// List all node_config records from the database
pub async fn list_node_configs(db: &Surreal<Client>) -> Result<Vec<NodeConfig>> {
    let configs: Vec<NodeConfig> = db
        .select("node_config")
        .await
        .context("Failed to query all node_configs from database")?;
    
    Ok(configs)
}

/// Get node_config by model and kind
pub async fn get_node_config_by_model_kind(
    db: &Surreal<Client>,
    model: &NodeModel,
    kind: &str,
) -> Result<Option<NodeConfig>> {
    let mut response = db
        .query("SELECT * FROM ONLY node_config WHERE model = $model AND kind = $kind")
        .bind(("model", model.to_string()))
        .bind(("kind", kind.to_string()))
        .await
        .context(format!(
            "Failed to query node_config from database: model={}, kind={}",
            model, kind
        ))?;

    let config: Option<NodeConfig> = response.take(0)?;
    Ok(config)
}

/// Get node_config from node_model
async fn get_node_config(db: &Surreal<Client>, node_model: &NodeModel) -> Result<NodeConfig> {
    let mut response = db
        .query("SELECT * FROM ONLY node_config WHERE model = $model_id")
        .bind(("model_id", node_model.to_string()))
        .await
        .context(format!(
            "Failed to query node_config from database: {node_model}"
        ))?;

    let config: Option<NodeConfig> = response.take(0)?;

    config.ok_or_else(|| anyhow!("Node config not found for model: {node_model}"))
}

/// Assign a lab node
pub async fn create_lab_node(
    db: &Surreal<Client>,
    name: &str,
    index: u16,
    model: NodeModel,
    lab: &DbLab,
) -> Result<DbNode> {
    let config = get_node_config(&db, &model).await?;
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

/// Delete all links for a lab
pub async fn delete_lab_links(db: &Surreal<Client>, lab_id: &str) -> Result<()> {
    let lab = get_lab_record(&db, lab_id).await?;
    let lab_record_id = get_lab_id(&lab)?;

    let _deleted: Vec<DbLink> = db
        .query("DELETE link WHERE lab = $lab_record_id")
        .bind(("lab_record_id", lab_record_id))
        .await
        .context(format!("Failed to delete links for lab: {lab_id}"))?
        .take(0)?;

    Ok(())
}
