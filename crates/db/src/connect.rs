use std::sync::Arc;

use anyhow::{Context, Result};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;

use shared::konst::SHERPA_DB_USER;

pub async fn connect(
    host: &str,
    port: u16,
    namespace: &str,
    database: &str,
    password: &str,
) -> Result<Arc<Surreal<Client>>> {
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

    Ok(Arc::new(db))
}
