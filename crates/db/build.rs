//! Checks every SQL statement in `sql/queries/` against the schema before the crate compiles.
//!
//! The `CREATE TABLE` statements in `sql/schema/` are applied to an in-memory Turso database, and
//! each query file is prepared against it. Preparing makes Turso check the statement's syntax and
//! every table and column it names, so a statement that does not fit the schema fails the build
//! with the file and Turso's message. For each file it generates a constant holding the statement,
//! in a module per directory: `sql/queries/user/find_by_id.sql` is `queries::user::FIND_BY_ID`.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use turso::{Builder, Connection};

const SQL_DIR: &str = "sql";
const SCHEMA_DIR: &str = "sql/schema";
const QUERIES_DIR: &str = "sql/queries";
const GENERATED_FILE: &str = "queries.rs";

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed={SQL_DIR}");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .context("Failed to start a runtime for checking SQL")?;
    let generated = runtime.block_on(check_queries(&manifest_dir))?;
    fs::write(out_dir.join(GENERATED_FILE), generated)
        .context("Failed to write the generated queries")?;
    Ok(())
}

/// Apply the schema, prepare every query file, and return the generated constants.
async fn check_queries(manifest_dir: &Path) -> Result<String> {
    let db = Builder::new_local(":memory:")
        .build()
        .await
        .context("Failed to open an in-memory database for checking SQL")?;
    let conn = db.connect()?;
    for file in sql_files(&manifest_dir.join(SCHEMA_DIR))? {
        let schema = fs::read_to_string(&file)?;
        conn.execute_batch(&schema)
            .await
            .with_context(|| format!("Failed to apply {}", file.display()))?;
    }

    let mut errors = Vec::new();
    let generated = module(
        &conn,
        manifest_dir,
        &manifest_dir.join(QUERIES_DIR),
        &mut errors,
    )
    .await?;
    if !errors.is_empty() {
        bail!(
            "SQL statements do not fit the schema:\n{}",
            errors.join("\n")
        );
    }
    Ok(generated)
}

/// The constants for the query files in `dir`, and a nested module for each subdirectory.
async fn module(
    conn: &Connection,
    manifest_dir: &Path,
    dir: &Path,
    errors: &mut Vec<String>,
) -> Result<String> {
    let mut code = String::new();
    for file in sql_files(dir)? {
        let sql = fs::read_to_string(&file)?;
        let relative = file.strip_prefix(manifest_dir)?.display().to_string();
        if let Err(error) = conn.prepare(&sql).await {
            errors.push(format!("  {relative}: {error}"));
        }
        let name = file
            .file_stem()
            .context("Query file has no name")?
            .to_string_lossy()
            .to_uppercase();
        writeln!(code, "/// `{relative}`")?;
        writeln!(
            code,
            "pub(crate) const {name}: &str = include_str!({:?});",
            file.display().to_string()
        )?;
    }
    let mut dirs: Vec<PathBuf> = fs::read_dir(dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    dirs.retain(|path| path.is_dir());
    dirs.sort();
    for sub in dirs {
        let name = sub
            .file_name()
            .context("Query directory has no name")?
            .to_string_lossy()
            .to_string();
        let inner = Box::pin(module(conn, manifest_dir, &sub, errors)).await?;
        writeln!(code, "pub(crate) mod {name} {{\n{inner}}}")?;
    }
    Ok(code)
}

/// The `.sql` files directly in `dir`, sorted by name.
fn sql_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| format!("Failed to read {}", dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    files.retain(|path| path.extension().is_some_and(|ext| ext == "sql"));
    files.sort();
    Ok(files)
}
