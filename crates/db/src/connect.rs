use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;

pub async fn connect(
    host: &str,
    port: u16,
    namespace: &str,
    database: &str,
) -> surrealdb::Result<Arc<Surreal<Client>>> {
    // docker run --rm -d --pull always --name surrealdb -p 8000:8000 surrealdb/surrealdb:v3.0.0-beta.2 start --log trace --user sherpa --pass Everest1953! memory

    let db = Surreal::new::<Ws>(format!("{host}:{port}/rpc")).await?;

    // Sign in as root
    db.signin(Root {
        username: "sherpa".to_string(),
        password: "Everest1953!".to_string(),
    })
    .await?;

    // Select namespace and database
    db.use_ns(namespace).use_db(database).await?;

    Ok(Arc::new(db))
}
