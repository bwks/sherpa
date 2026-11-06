//! Checks every SQL statement in `sql/queries/` against the schema before the crate compiles.
//!
//! The `CREATE TABLE` statements in `sql/schema/` are schema version 1. They are applied to an
//! in-memory Turso database, followed by each migration in `sql/migrations/`, so the database has
//! the schema a running Sherpa has. Each query file is then prepared against it. Preparing makes
//! Turso check the statement's syntax and every table and column it names, so a statement that
//! does not fit the schema fails the build with the file and Turso's message. A statement that
//! returns every column of a table must return them in the table's order, as the row mappers in
//! `src/persistence.rs` read them by position.
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
/// The files in `sql/schema/`, parents before children, in the order `src/schema/apply.rs`
/// applies them.
const SCHEMA_FILES: &[&str] = &[
    "user.sql",
    "node_image.sql",
    "lab.sql",
    "node.sql",
    "link.sql",
    "bridge.sql",
];

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
    let schema_dir = manifest_dir.join(SCHEMA_DIR);
    let mut found: Vec<String> = sql_files(&schema_dir)?
        .iter()
        .filter_map(|file| {
            file.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .collect();
    found.sort();
    let mut listed: Vec<String> = SCHEMA_FILES.iter().map(ToString::to_string).collect();
    listed.sort();
    if found != listed {
        bail!("{SCHEMA_DIR} holds {found:?}, but SCHEMA_FILES lists {listed:?}");
    }
    for name in SCHEMA_FILES {
        let file = schema_dir.join(name);
        let schema = fs::read_to_string(&file)?;
        conn.execute_batch(&schema)
            .await
            .with_context(|| format!("Failed to apply {}", file.display()))?;
    }
    let migrations = migrations(&conn, manifest_dir).await?;
    let tables = table_columns(&conn).await?;

    let mut errors = Vec::new();
    let queries = module(
        &conn,
        manifest_dir,
        &manifest_dir.join(QUERIES_DIR),
        &tables,
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

/// Each table and its columns, in the table's order.
async fn table_columns(conn: &Connection) -> Result<Vec<(String, Vec<String>)>> {
    let mut names = Vec::new();
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            (),
        )
        .await?;
    while let Some(row) = rows.next().await? {
        names.push(row.get::<String>(0)?);
    }
    let mut tables = Vec::new();
    for table in names {
        let mut columns = Vec::new();
        let mut rows = conn
            .query(&format!("PRAGMA table_info({table})"), ())
            .await?;
        while let Some(row) = rows.next().await? {
            columns.push(row.get::<String>(1)?);
        }
        tables.push((table, columns));
    }
    Ok(tables)
}

/// The constants for the query files in `dir`, and a nested module for each subdirectory.
async fn module(
    conn: &Connection,
    manifest_dir: &Path,
    dir: &Path,
    tables: &[(String, Vec<String>)],
    errors: &mut Vec<String>,
) -> Result<String> {
    let mut code = String::new();
    for file in sql_files(dir)? {
        let sql = fs::read_to_string(&file)?;
        let relative = file.strip_prefix(manifest_dir)?.display().to_string();
        match conn.prepare(&sql).await {
            Ok(statement) => {
                let returned = statement.column_names();
                for (table, columns) in tables {
                    let mut sorted = returned.clone();
                    sorted.sort();
                    let mut expected = columns.clone();
                    expected.sort();
                    if sorted == expected && returned != *columns {
                        errors.push(format!(
                            "  {relative}: returns every column of {table}, but not in its \
                             order: {}",
                            columns.join(", ")
                        ));
                    }
                }
            }
            Err(error) => errors.push(format!("  {relative}: {error}")),
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
        let inner = Box::pin(module(conn, manifest_dir, &sub, tables, errors)).await?;
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
