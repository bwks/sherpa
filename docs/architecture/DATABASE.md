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
- **Whole operations under the lock.** A write that reads before it writes does both under one
  `Writer`, so nothing changes between the check and the write. Statements that must succeed
  together run in one transaction (`writer.conn.transaction()`, committed explicitly; dropping it
  rolls back); a test checks that a transaction that fails after DDL and a `user_version` change
  leaves neither.
- **Narrow updates.** An update writes only the columns it is for: `update_lab_state`,
  `update_lab_tailscale_container`, `update_link_impairments` and the `update_node_*` functions.
  A caller holding an older copy of the record cannot write back columns another task changed
  meanwhile. `update_user` writes the user's own fields, sets `updated_at` itself and never changes
  `created_at`. A link is created with its impairments in one statement (`create_link` takes a
  `DbLink`).
- **Exclusive file.** While `sherpad` has the file open, no other process can open it, including
  the `sqlite3` command; Turso's multi-process mode is experimental. Inspection therefore runs
  with `sherpad` stopped.
- **Backups.** `Database::backup(path)` runs `VACUUM INTO` under the write lock, so the copy is
  consistent and writes wait for it. It refuses a destination that exists. The `server.backup`
  RPC (`crates/server/src/services/backup.rs`) writes to `SHERPA_DB_BACKUP_PATH`, which it first
  restricts to `sherpa` (`SHERPA_DB_BACKUP_DIR_MODE`, `0700`): `VACUUM INTO` writes the copy before
  its file is restricted. The database directory is `0750` (`SHERPA_DB_DIR_MODE`), set by the
  installer, the update script and `sherpad` at startup. `open` restricts the database file and
  its `-wal` and `-shm` files to `0600`; Turso creates the `-wal` file when it opens the database,
  so the first writes do not leave it readable by others.
- **Tests.** `open_in_memory()` gives each test its own database; connections to one in-memory
  database share its data. The db and server integration tests need no database server.

## Queries and row mapping

Every statement the crate runs is a plain SQL file under `crates/db/sql/queries/<table>/`, one
statement per file, complete and readable. The tables as first created are
`crates/db/sql/schema/<table>.sql`, which the schema code includes with `include_str!`; each later
schema change is a file in `crates/db/sql/migrations/` (see [Versions](#versions)).

```mermaid
flowchart LR
    schema["sql/schema/*.sql"] --> build["build.rs"]
    migrations["sql/migrations/*.sql"] --> build
    queries["sql/queries/&lt;table&gt;/*.sql"] --> build
    build -->|"applies the schema and every migration to an in-memory Turso database, prepares every query"| out["OUT_DIR/queries.rs"]
    build --> mig["OUT_DIR/migrations.rs"]
    out -->|"include!"| mod["queries::lab::FIND_BY_ID, ..."]
    mig -->|"include!"| apply["schema::migrations::ALL"]
```

- **Checked at build time.** `crates/db/build.rs` applies the schema files, then each migration in
  order, to an in-memory Turso database, so it has the schema a running server has, and prepares
  each query file against it. Preparing checks the syntax and every table and
  column a statement names, so a statement that does not fit the schema fails the build, naming the
  file and Turso's message. It reruns only when a file under `sql/` changes, and takes about a
  second; Cargo shares the `turso` build with the crate, so a cold build compiles Turso once and
  the script adds about 14 seconds.
- **Constants.** The build script generates one constant per file, holding the file with
  `include_str!`, in a module per directory: `sql/queries/lab/find_by_id.sql` is
  `queries::lab::FIND_BY_ID`. A statement used by more than one module, such as
  `queries::node::COUNT_BY_LAB`, is one file. Triggers are migrations. Only `PRAGMA
  user_version`, which takes no bound parameter, is built in `schema/apply.rs`.
- **Column order checked.** A statement that returns every column of a table must return them in
  the table's order (`PRAGMA table_info`), or the build fails naming the file: the mappers read by
  position, and two columns of one type, such as `gateway_ipv4` and `router_ipv4`, would otherwise
  swap silently. A statement returning some columns, such as the network readers' `lab_id` and
  network, is not checked, nor is a statement's parameter count.
- **Schema order.** The build script applies `sql/schema/` in the order `apply_schema` does,
  parents first (`SCHEMA_FILES`), and refuses a schema file missing from that list.

`crates/db/src/sql.rs` has the three query helpers: `query_optional`, `query_all` and
`query_count`. The first two take a mapper, a plain `fn(&Row) -> Result<T>`. A failed statement
is named by its first line, and a row that does not map by its first column, its id.

`crates/db/src/persistence.rs` maps rows to records:

- **Column order.** Statements list a table's columns in its `CREATE TABLE` order, and the mappers
  read and write them in that order.
- **Reading.** `Cells` reads a row's columns in order and names the field in every error.
  `*_from_row` builds a record; `*_values` gives a record's values in column order, for
  positional parameters (`?1`, `?2`, ...). SQL never contains formatted values.
- **Types.** Unit enums are stored as text, by their serde names (`enum_text`). Lists such as
  `ssh_keys` and `cpu_pinning` are JSON text. Timestamps are RFC 3339 text from `jiff`, which keeps
  nanoseconds. Booleans are `0` and `1`. A link's impairments are flattened into `a_to_b_*` and
  `b_to_a_*` columns, and a bridge's members are rows of `bridge_member`, ordered by `position`.
- **Record keys.** A column holds only a record's key; the table is implied by the column, and
  `Cells::record(table, field)` rebuilds the `RecordId`. `key_of` refuses an id from another table.
  New keys are 20 random characters from `SHERPA_DB_KEY_CHARSET`, the length and alphabet of the
  keys of releases before 0.5.0, which upgraded servers still hold.

## Schema

`crates/db/src/schema/` has one file per table. Tables are `STRICT`. Ranges are `CHECK`
constraints; uniqueness is `UNIQUE` constraints; references are foreign keys:

- `lab.user` and `node.image` are `ON DELETE RESTRICT`: an owner with labs, or an image in use,
  cannot be deleted.
- `node.lab`, `link.lab`, `link.node_a`, `link.node_b`, `bridge.lab`, `bridge_member.bridge` and
  `bridge_member.node` are `ON DELETE CASCADE`. Deleting a lab is one `DELETE`.

### Versions

`apply_schema` (`schema/apply.rs`) runs on every start and in `sherpad init`, in one transaction.
The database's `PRAGMA user_version` is its schema version. Version 1 is the tables in
`sql/schema/` (`TABLES` in `apply.rs`). Each file in `sql/migrations/` takes the schema up one
version; its name is the version it takes the schema to, four digits, then a name:
`0002_fix_data_interface_counts.sql`. The build script refuses a gap or a misnumbered file, and
generates `schema::migrations`: a constant per file, named without the number
(`FIX_DATA_INTERFACE_COUNTS`), and `ALL`, every migration in order. `SCHEMA_VERSION` is
`1 + ALL.len()`.
A database with a newer version than the build knows is refused, so a downgrade cannot write to a
schema it does not understand.

A version never changes once released, including `sql/schema/`. To change the schema, add the
next migration file, and change the queries that use it; the build checks them against the new
schema. New columns come last, so the mappers in `persistence.rs` read and write them last. SQLite
cannot alter
most constraints in place, so a change to a column's constraints is a table rebuild: create the new
table, copy the rows, drop the old one, rename.

| Version | Change |
|---|---|
| 1 | Every table (`sql/schema/<table>.sql`) |
| 2 | `0002_fix_data_interface_counts.sql`: `cisco_csr1000v` and `cisco_iosvl2` images stored with more than 15 data interfaces (the old default was 16, one more than they have after the management interface) are set to 15 |
| 3 | `0003_lab_source.sql`: `lab.client_host` and `lab.client_folder`, where `sherpa up` ran (see below) |
| 4 | `0004_fix_ztp_image_defaults.sql`: `cisco_ise` images stored with the CD-ROM ZTP method (the old default, which has no ISE generator) are set to the disk method, and `generic_vm` images stored with a VirtIO CD-ROM bus (the old default, which cannot hold the cloud-init ISO) are set to SATA |
| 5 | `0005_fix_vsrx_ztp_method.sql`: `juniper_vsrxv3` images stored with the TFTP ZTP method (the old default; vSRX's factory config runs no DHCP client, so it never fetched its config) are set to the CD-ROM method |
| 6 | `0006_one_default_node_image.sql`: one default image per model and kind. Duplicate defaults keep the most recently written image, then the partial unique index `node_image_one_default` refuses a second |
| 7 | `0007_interfaces_used_once.sql`: triggers refuse a link or bridge member on a node interface already used by a link or bridge, a link from an interface to itself, and an interface twice in one bridge. They check new rows only, so a database that holds a repeat still opens |
| 8 | `0008_static_triggers.sql`: drops the enum rule triggers, which earlier releases rebuilt from the Rust enums on every start, and creates the admin protection triggers as fixed text |
| 9 | `0009_drop_dedicated_management_interface.sql`: drops `node_image.dedicated_management_interface`. Index 0 of every model's interface table is its management interface, so the flag changed nothing |
| 10 | `0010_lab_description.sql`: `lab.description`, the manifest's optional `description` (see below) |
| 11 | `0011_lab_ssh_key.sql`: `lab.ssh_key`, the key pair the `sherpa` account on the lab's nodes logs in with: `lab` for the lab's own, `server` (the default, for every lab created before) for the server's shared key. See [`SERVER.md`](SERVER.md#lab-ssh-keys) |

A migration can correct data as well as change the schema; it runs once, when a database reaches that
version.

### What the user said about a lab

`lab.description` holds the manifest's optional `description`. `topology::validate::check_lab_description`,
which the client and the server both run, refuses one over 1000 characters or with control
characters other than line breaks and tabs.

`lab.client_host` and `lab.client_folder` record the hostname of the machine `sherpa up` ran on
and the folder it ran in. The CLI sends them in the `up` request (`client_host`, `client_folder`);
`server::validate::validate_lab_origin` refuses an empty value, one over its length limit (253 and
4096 characters) or one with control characters. Labs started from the web UI or the API, and
labs created before version 3, have neither (`NULL`).

`create_lab` stores all three through `db::LabDetails`. `inspect` reads them from the record into
`LabInfo`, so the CLI's lab table and the web UI's lab page show them. The description is also in
each lab's `LabSummary`, so the web UI's labs tables and the `labs.list` API show it too.

### Enum values and triggers

Enum columns are checked by the types, not by the database: every value `db` writes is serialized
from its Rust enum (`enum_text`), and every value it reads is decoded back, so a value outside the
enum, such as one a newer release stored before a downgrade, fails when its row is read, naming the
row. Data-fix migrations, which write enum names as literal text, each have a test that reads the
corrected rows back.

Triggers enforce only rules the types cannot express. All of them are fixed text in migrations, so
the build checks them:

- **The built-in admin account** (`0008_static_triggers.sql`). Triggers refuse deleting, renaming
  or demoting `admin`, and an `admin` row without the admin flag, with `ADMIN_PROTECTED_MESSAGE`.
  They apply to any statement, not only the application's.
- **Interfaces used once** (`0007_interfaces_used_once.sql`).

## Upgrades from before 0.5.0

Releases before 0.5.0 kept their data in a SurrealDB container. Releases 0.5.0 to 0.5.12 moved it
into the embedded database with `sherpad migrate-db`, which `scripts/sherpa_update.sh` ran on the
first update. Later releases have no importer: the update script refuses such a server before
changing anything and tells the user to update to 0.5.12 first.
