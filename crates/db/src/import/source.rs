//! Reading a pre-0.5.0 server's SurrealDB.
use std::fmt;

use anyhow::{Context, Result};
use shared::data::{DbBridge, DbLab, DbLink, DbNode, DbUser, NodeConfig};
use shared::konst::{
    SHERPA_DB_NAME, SHERPA_DB_NAMESPACE, SHERPA_DB_PORT, SHERPA_DB_SERVER, SHERPA_DB_USER,
};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use tracing::instrument;

use super::rows::{BridgeRow, LabRow, LinkRow, NodeImageRow, NodeRow, UserRow};

/// Where the SurrealDB server of a pre-0.5.0 installation listens, and how to sign in to it.
#[derive(Clone)]
pub struct SurrealSource {
    pub host: String,
    pub port: u16,
    pub namespace: String,
    pub database: String,
    pub password: String,
}

/// Every record read from SurrealDB, converted to Sherpa's records.
#[derive(Debug, Default)]
pub(super) struct SurrealData {
    pub users: Vec<DbUser>,
    pub node_images: Vec<NodeConfig>,
    pub labs: Vec<DbLab>,
    pub nodes: Vec<DbNode>,
    pub links: Vec<DbLink>,
    pub bridges: Vec<DbBridge>,
}

impl SurrealSource {
    /// The server a pre-0.5.0 installation ran: on this host, with Sherpa's namespace and database.
    pub fn local(password: String) -> Self {
        Self {
            host: SHERPA_DB_SERVER.to_string(),
            port: SHERPA_DB_PORT,
            namespace: SHERPA_DB_NAMESPACE.to_string(),
            database: SHERPA_DB_NAME.to_string(),
            password,
        }
    }
}

// The password never appears in logs or errors.
impl fmt::Debug for SurrealSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SurrealSource")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("namespace", &self.namespace)
            .field("database", &self.database)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Connect and sign in to the SurrealDB server as Sherpa's root user.
#[instrument(level = "debug")]
pub(super) async fn connect(source: &SurrealSource) -> Result<Surreal<Client>> {
    let db = Surreal::new::<Ws>(format!("{}:{}/rpc", source.host, source.port))
        .await
        .context(format!(
            "Failed to connect to SurrealDB at {}:{}",
            source.host, source.port
        ))?;
    db.signin(Root {
        username: SHERPA_DB_USER.to_string(),
        password: source.password.clone(),
    })
    .await
    .context("There was a problem with SurrealDB authentication")?;
    db.use_ns(&source.namespace)
        .use_db(&source.database)
        .await
        .context("Failed to select the SurrealDB namespace and database")?;
    Ok(db)
}

/// Read every table Sherpa used.
#[instrument(skip(db), level = "debug")]
pub(super) async fn read_all(db: &Surreal<Client>) -> Result<SurrealData> {
    let users: Vec<UserRow> = db
        .select("user")
        .await
        .context("Failed to read users from SurrealDB")?;
    let node_images: Vec<NodeImageRow> = db
        .select("node_image")
        .await
        .context("Failed to read node images from SurrealDB")?;
    let labs: Vec<LabRow> = db
        .select("lab")
        .await
        .context("Failed to read labs from SurrealDB")?;
    let nodes: Vec<NodeRow> = db
        .select("node")
        .await
        .context("Failed to read nodes from SurrealDB")?;
    let links: Vec<LinkRow> = db
        .select("link")
        .await
        .context("Failed to read links from SurrealDB")?;
    let bridges: Vec<BridgeRow> = db
        .select("bridge")
        .await
        .context("Failed to read bridges from SurrealDB")?;

    Ok(SurrealData {
        users: users
            .into_iter()
            .map(DbUser::try_from)
            .collect::<Result<_>>()
            .context("Failed to convert SurrealDB users")?,
        node_images: node_images
            .into_iter()
            .map(NodeConfig::try_from)
            .collect::<Result<_>>()
            .context("Failed to convert SurrealDB node images")?,
        labs: labs
            .into_iter()
            .map(DbLab::try_from)
            .collect::<Result<_>>()
            .context("Failed to convert SurrealDB labs")?,
        nodes: nodes
            .into_iter()
            .map(DbNode::try_from)
            .collect::<Result<_>>()
            .context("Failed to convert SurrealDB nodes")?,
        links: links
            .into_iter()
            .map(DbLink::try_from)
            .collect::<Result<_>>()
            .context("Failed to convert SurrealDB links")?,
        bridges: bridges
            .into_iter()
            .map(DbBridge::try_from)
            .collect::<Result<_>>()
            .context("Failed to convert SurrealDB bridges")?,
    })
}
