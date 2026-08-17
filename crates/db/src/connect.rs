use std::ops::Deref;
use std::sync::Arc;

use anyhow::{Context, Result};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use tracing::instrument;

use shared::konst::SHERPA_DB_USER;

/// Shared database connection handle.
///
/// The concrete SurrealDB client stays owned and named by this crate so callers
/// do not need a direct SurrealDB dependency.
#[derive(Clone)]
pub struct Database(Arc<Surreal<Client>>);

impl Deref for Database {
    type Target = Arc<Surreal<Client>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[instrument(skip(password), level = "debug")]
pub async fn connect(
    host: &str,
    port: u16,
    namespace: &str,
    database: &str,
    password: &str,
) -> Result<Database> {
    let db = Surreal::new::<Ws>(format!("{host}:{port}/rpc"))
        .await
        .context("Failed to connect to SurrealDB")?;

    db.signin(Root {
        username: SHERPA_DB_USER.to_string(),
        password: password.to_string(),
    })
    .await
    .context("There was a problem with database authentication")?;

    db.use_ns(namespace)
        .use_db(database)
        .await
        .context("Failed to select namespace and database")?;

    Ok(Database(Arc::new(db)))
}
