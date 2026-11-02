use std::path::Path;

use anyhow::{Context, Result};
use tracing::instrument;

use db::{SurrealSource, import_from_surrealdb, open};
use shared::konst::{
    SHERPA_DB_FILE_PATH, SHERPA_DB_NAME, SHERPA_DB_NAMESPACE, SHERPA_DB_PATH, SHERPA_DB_SERVER,
};
use shared::util::{create_dir, render_migration_table, term_msg_surround};

/// Import the data of a server older than 0.5.0 from its SurrealDB container into the embedded
/// database.
///
/// Run it once, with `sherpad` stopped (it holds the database file) and the SurrealDB container
/// still running. The import runs in one transaction and refuses a database that already holds
/// users or labs, so a failed run leaves nothing behind and can be repeated.
///
/// The password and port come from flags or the environment. The update script runs this command
/// through systemd with the service's env file, so it reads the same values the old service did.
#[instrument(skip(db_password))]
pub async fn migrate_db(db_password: &str, port: u16) -> Result<()> {
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
        password: db_password.to_string(),
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
