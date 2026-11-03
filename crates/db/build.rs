//! Checks every SQL statement in `sql/queries/` against the schema before the crate compiles.
//!
//! The `CREATE TABLE` statements in `sql/schema/` are schema version 1. They are applied to an
//! in-memory Turso database, followed by each migration in `sql/migrations/`, so the database has
//! the schema a running Sherpa has. Each query file is then prepared against it. Preparing makes
//! Turso check the statement's syntax and every table and column it names, so a statement that
//! does not fit the schema fails the build with the file and Turso's message.
//!
//! It generates two files:
//! - `queries.rs`: a constant for each query file, in a module per directory:
//!   `sql/queries/user/find_by_id.sql` is `queries::user::FIND_BY_ID`.
//! - `migrations.rs`: a constant for each migration, named without its number, and `ALL`, every
//!   migration in order. `sql/migrations/0002_fix_data_interface_counts.sql` takes the schema to
//!   version 2 and is `FIX_DATA_INTERFACE_COUNTS`.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use turso::{Builder, Connection};

const SQL_DIR: &str = "sql";
const SCHEMA_DIR: &str = "sql/schema";
const MIGRATIONS_DIR: &str = "sql/migrations";
const QUERIES_DIR: &str = "sql/queries";
const QUERIES_FILE: &str = "queries.rs";
const MIGRATIONS_FILE: &str = "migrations.rs";
/// The schema version the first migration takes the database to; `sql/schema/` is version 1.
const FIRST_MIGRATION_VERSION: usize = 2;

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed={SQL_DIR}");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .context("Failed to start a runtime for checking SQL")?;
    let (queries, migrations) = runtime.block_on(check_sql(&manifest_dir))?;
    fs::write(out_dir.join(QUERIES_FILE), queries)
        .context("Failed to write the generated queries")?;
    fs::write(out_dir.join(MIGRATIONS_FILE), migrations)
        .context("Failed to write the generated migrations")?;
    Ok(())
}

/// Apply the schema and migrations, prepare every query file, and return the generated queries
/// and migrations.
async fn check_sql(manifest_dir: &Path) -> Result<(String, String)> {
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
    let migrations = migrations(&conn, manifest_dir).await?;

    let mut errors = Vec::new();
    let queries = module(
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
    Ok((queries, migrations))
}

/// Apply each migration in order, and return their constants. A migration's file name is its
/// target version, four digits, then its name: `0002_fix_data_interface_counts.sql`.
async fn migrations(conn: &Connection, manifest_dir: &Path) -> Result<String> {
    let mut code = String::new();
    let mut names = Vec::new();
    for (file, version) in sql_files(&manifest_dir.join(MIGRATIONS_DIR))?
        .into_iter()
        .zip(FIRST_MIGRATION_VERSION..)
    {
        let relative = file.strip_prefix(manifest_dir)?.display().to_string();
        let stem = file
            .file_stem()
            .context("Migration file has no name")?
            .to_string_lossy()
            .to_string();
        let expected = format!("{version:04}_");
        let Some(name) = stem.strip_prefix(&expected) else {
            bail!("{relative}: expected the name to start with {expected}, its schema version");
        };
        let sql = fs::read_to_string(&file)?;
        conn.execute_batch(&sql)
            .await
            .with_context(|| format!("Failed to apply {relative}"))?;
        let name = name.to_uppercase();
        writeln!(code, "/// `{relative}`: schema version {version}.")?;
        writeln!(
            code,
            "pub(crate) const {name}: &str = include_str!({:?});",
            file.display().to_string()
        )?;
        names.push(name);
    }
    writeln!(
        code,
        "/// Every migration in order; entry `i` takes the schema to version `i + \
         {FIRST_MIGRATION_VERSION}`."
    )?;
    writeln!(
        code,
        "pub(crate) const ALL: &[&str] = &[{}];",
        names.join(", ")
    )?;
    Ok(code)
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
