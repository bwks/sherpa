use std::path::Path;

use anyhow::{Context, Result, anyhow};
use tracing::instrument;

use db::{SurrealSource, import_from_surrealdb, open};
use shared::konst::{
    SHERPA_DB_FILE_PATH, SHERPA_DB_NAME, SHERPA_DB_NAMESPACE, SHERPA_DB_PATH, SHERPA_DB_PORT,
    SHERPA_DB_SERVER, SHERPA_ENV_FILE_PATH,
};
use shared::util::{create_dir, read_env_file_value, render_migration_table, term_msg_surround};

/// Import the data of a server older than 0.5.0 from its SurrealDB container into the embedded
/// database.
///
/// Run it once, with `sherpad` stopped (it holds the database file) and the SurrealDB container
/// still running. The import runs in one transaction and refuses a database that already holds
/// users or labs, so a failed run leaves nothing behind and can be repeated.
#[instrument(skip(db_password))]
pub async fn migrate_db(db_password: Option<&str>, db_port: Option<u16>) -> Result<()> {
    let env_file = Path::new(SHERPA_ENV_FILE_PATH);

    let password = match db_password {
        Some(password) => password.to_string(),
        None => read_env_file_value(env_file, "SHERPA_DB_PASSWORD").ok_or_else(|| {
            anyhow!(
                "SurrealDB password not provided. Supply it via:\n  \
                     1. --db-pass flag\n  \
                     2. SHERPA_DB_PASSWORD environment variable\n  \
                     3. SHERPA_DB_PASSWORD entry in {}",
                env_file.display()
            )
        })?,
    };

    let port = match db_port {
        Some(port) => port,
        None => read_env_file_value(env_file, "SHERPA_DB_PORT")
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(SHERPA_DB_PORT),
    };

    term_msg_surround("Sherpa Database Migration");

    create_dir(SHERPA_DB_PATH)?;
    let target = open(Path::new(SHERPA_DB_FILE_PATH))
        .await
        .context("Failed to open the database; stop sherpad before migrating")?;

    let source = SurrealSource {
        host: SHERPA_DB_SERVER.to_string(),
        port,
        namespace: SHERPA_DB_NAMESPACE.to_string(),
        database: SHERPA_DB_NAME.to_string(),
        password,
    };
    let summary = import_from_surrealdb(&source, &target)
        .await
        .context("Database migration failed; nothing was imported")?;

    println!(
        "{}",
        render_migration_table(&[
            ("Users", summary.users),
            ("Node images", summary.node_images),
            ("Labs", summary.labs),
            ("Nodes", summary.nodes),
            ("Links", summary.links),
            ("Bridges", summary.bridges),
            ("Bridge members", summary.bridge_members),
        ])
    );
    println!("Imported into {SHERPA_DB_FILE_PATH}");
    Ok(())
}
