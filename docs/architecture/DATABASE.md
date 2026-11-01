# Database

Sherpa's records (users, node images, labs, nodes, links, bridges) live in an embedded
[Turso](https://github.com/tursodatabase/turso) database inside `sherpad`: the `turso` crate,
pinned to an exact pre-1.0 version with default features off, because they install mimalloc as the
process's global allocator. The file is `SHERPA_DB_FILE_PATH`
(`/opt/sherpa/db/sherpa.sqlite`), in SQLite's format, with a write-ahead log. Everything is in
`crates/db`; no other crate depends on Turso. Outside the crate, records are the `shared::data`
types and ids are the database-neutral `shared::data::RecordId`.

Releases before 0.5.0 used a SurrealDB container. The spike that chose Turso, and its measurements,
are in [`../investigations/turso-migration.md`](../investigations/turso-migration.md).

## The `Database` handle

`db::open(path)` and `db::open_in_memory()` return a `Database` (`crates/db/src/connect.rs`). It is
`Clone`; `AppState` holds one and every task shares it.

- **A connection per operation.** A Turso `Connection` cannot be used by two tasks at once: a
  second concurrent use fails with `Misuse("concurrent use forbidden")`. So the handle holds the
  `turso::Database`, and each operation takes a fresh connection from `reader()` or `writer()`.
  Every connection gets a busy timeout (`SHERPA_DB_BUSY_TIMEOUT_SECS`) and
  `PRAGMA foreign_keys = ON`, which SQLite leaves off by default.
- **One writer at a time.** `writer()` also takes the handle's write lock, a `tokio::sync::Mutex`,
  and returns a `Writer` holding both until it is dropped. Turso's busy timeout works for a single
  waiter, but under contention its busy wait does not serve waiters in order: with 64 writing
  tasks, about 0.5% of transactions waited past the timeout and failed with `Busy`. The tokio mutex
  is fair, so writers queue in arrival order instead, with no loss of throughput. Reads take no
  lock.
- **Whole operations under the lock.** A write that reads before it writes, such as an update that
  checks an immutable field, does both under one `Writer`, so nothing changes between the check
  and the write. Statements that must succeed together run in one transaction
  (`writer.conn.transaction()`, committed explicitly; dropping it rolls back).
- **Exclusive file.** While `sherpad` has the file open, no other process can open it, including
  the `sqlite3` command; Turso's multi-process mode is experimental. `sherpad migrate-db` and any
  inspection therefore run with `sherpad` stopped.
- **Backups.** `Database::backup(path)` runs `VACUUM INTO` under the write lock, so the copy is
  consistent and writes wait for it. It refuses a destination that exists. The `server.backup`
  RPC (`crates/server/src/services/backup.rs`) writes to `SHERPA_DB_BACKUP_PATH`.
- **Tests.** `open_in_memory()` gives each test its own database; connections to one in-memory
  database share its data. The db and server integration tests need no database server.

## Queries and row mapping

`crates/db/src/sql.rs` has the three query helpers: `query_optional`, `query_all` and
`query_count`. The first two take a mapper, a plain `fn(&Row) -> Result<T>`.

`crates/db/src/persistence.rs` maps rows to records:

- **Column lists.** Each table has a column-list macro (`user_columns!()`, `lab_columns!()`, and
  so on). Queries build their SQL with `concat!` from the macro, so the `SELECT`, `INSERT ...
  RETURNING` and the mapper always agree on the column order, at compile time.
- **Reading.** `Cells` reads a row's columns in order and names the field in every error.
  `*_from_row` builds a record; `*_values` gives a record's values in column order, for
  positional parameters (`?1`, `?2`, ...). SQL never contains formatted values.
- **Types.** Unit enums are stored as text, by their serde names (`enum_text`). Lists such as
  `ssh_keys` and `cpu_pinning` are JSON text. Timestamps are RFC 3339 text from `jiff`, which keeps
  nanoseconds. Booleans are `0` and `1`. A link's impairments are flattened into `a_to_b_*` and
  `b_to_a_*` columns, and a bridge's members are rows of `bridge_member`, ordered by `position`.
- **Record keys.** A column holds only a record's key; the table is implied by the column, and
  `Cells::record(table, field)` rebuilds the `RecordId`. `key_of` refuses an id from another table.
  New keys are 20 random characters from `SHERPA_DB_KEY_CHARSET`, the length and alphabet
  SurrealDB used, so imported keys and new ones look alike.

## Schema

`crates/db/src/schema/` has one file per table. Tables are `STRICT`. Ranges are `CHECK`
constraints; uniqueness is `UNIQUE` constraints; references are foreign keys:

- `lab.user` and `node.image` are `ON DELETE RESTRICT`: an owner with labs, or an image in use,
  cannot be deleted.
- `node.lab`, `link.lab`, `link.node_a`, `link.node_b`, `bridge.lab`, `bridge_member.bridge` and
  `bridge_member.node` are `ON DELETE CASCADE`. Deleting a lab is one `DELETE`.

### Versions

`apply_schema` (`schema/apply.rs`) runs on every start and in `sherpad init`, in one transaction.
The database's `PRAGMA user_version` counts how many entries of `MIGRATIONS` it has; each entry is
a list of SQL batches taking the schema up one version. `SCHEMA_VERSION` is the number of entries.
A database with a newer version than the build knows is refused, so a downgrade cannot write to a
schema it does not understand.

An entry never changes once released. To change the schema, append an entry. SQLite cannot alter
most constraints in place, so a change to a column's constraints is a table rebuild: create the new
table, copy the rows, drop the old one, rename.

### Rule triggers

Two kinds of rule are triggers, dropped and created again by every `apply_schema` rather than
versioned:

- **Enum columns.** The set of values comes from the Rust enums (`NodeModel::to_vec()` and so on),
  which grow with every new node model. A `CHECK` constraint would need a table rebuild each time,
  so each table with enum columns gets `<table>_rules_insert` and `<table>_rules_update` triggers,
  built by `schema::helpers::enum_rule` and `rule_triggers`. A value outside the list fails with
  `Invalid value for <table>.<column>`.
- **The built-in admin account** (`schema/user.rs`). Triggers refuse deleting, renaming or demoting
  `admin`, and an `admin` row without the admin flag, with `ADMIN_PROTECTED_MESSAGE`. They apply to
  any statement, not only the application's.

## Importer

`crates/db/src/import/` moves a pre-0.5.0 server's data. `sherpad migrate-db`
(`crates/server/src/migrate.rs`) calls `import_from_surrealdb(&SurrealSource, &Database)`, and
`scripts/sherpa_update.sh` runs it on the first update to 0.5.0.

- `source.rs` connects to the old SurrealDB container over WebSocket and reads every table.
- `rows.rs` holds the rows as SurrealDB stored them, and their conversion to Sherpa's records,
  including the old link impairment and bridge member layouts.
- `write.rs` applies the schema, then in one transaction under the writer refuses a target that
  already has a user or a lab, inserts every record with its original key in dependency order, and
  compares each table's row count with what it read. Any failure rolls back.

The importer is the only user of the `surrealdb` crate and of the `SHERPA_DB_SERVER`,
`SHERPA_DB_PORT`, `SHERPA_DB_NAMESPACE`, `SHERPA_DB_NAME` and `SHERPA_DB_USER` constants. They go
when the importer is removed.
