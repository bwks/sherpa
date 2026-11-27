# Database

Sherpa's records live in an embedded [Turso](https://github.com/tursodatabase/turso) database
inside `sherpad`, in SQLite's file format, all of it in `crates/db`. `turso` is pinned to an exact
pre-1.0 version with default features off, as they install mimalloc as the global allocator; the
`agent-bus` tool shares only the pin. Outside the crate, records are `shared::data` types. The behaviour is in `tests/specs/install-and-update.md`
(`schema_migration`, `database_key`, `database_backup`, `upgraded_records`).

## The `Database` handle

- **A connection per operation.** A Turso `Connection` refuses concurrent use from two tasks, so
  the handle holds the `turso::Database` and each operation takes a fresh connection. Every
  connection turns on `foreign_keys`, which SQLite leaves off.
- **One writer at a time.** `writer()` takes the handle's `tokio::sync::Mutex` as well. Turso's busy
  wait does not serve waiters in order, so under contention some writers waited past the busy
  timeout and failed; the fair mutex queues them in arrival order at no cost in throughput. Reads
  take no lock.
- **Whole operations under the lock.** A write that reads first does both under one `Writer`, and
  statements that must succeed together share a transaction, so nothing changes between a check
  and its write.
- **Narrow updates.** An update writes only its own columns, so a caller holding an old copy of a
  record cannot write back what another task changed. `update_user` is also compare-and-set on
  `updated_at`, and fails with `UserChanged` rather than undo a concurrent edit.
- **Exclusive file.** No other process, `sqlite3` included, opens the file while `sherpad` has it.
- **Backups.** `Database::backup` runs `VACUUM INTO` under the write lock, so the copy is
  consistent. It holds everything but the database key, which must be backed up beside it but
  apart from it, or the restored database will not start (see [Secret values](#secret-values)).
- **Update backups.** `sherpa_update.sh` copies the database file, its `-wal` and `-shm` files
  and the key, with sherpad stopped, to a new root-only folder in `/var/backups/sherpa` before
  the new release first opens them. When that release does not start it removes the `-wal` and
  `-shm` files the release left, as SQLite would apply them to the restored file, and puts the
  copies back (`update_script`). The update never removes the folder; `sherpa_uninstall.sh
  --remove-all` does, and the other uninstall modes keep it (`uninstall_script`).

## Queries and row mapping

Every statement is one SQL file under `sql/queries/<table>/`. `crates/db/build.rs` applies
`sql/schema/` and every migration to an in-memory database, so it has the schema a running server
has, and prepares each query against it: a statement that names a missing table or column fails the
build. It also generates one `Query<N>` constant per file, where `N` is the number of numbered
parameters (`?1`, `?2`, ...; a bare `?`, a named parameter or a gap fails the build), and
`Query::bind` takes exactly `[Value; N]`, so a wrong count is a type error at the call site.

The mappers in `persistence.rs` read and write columns by position, so a statement that returns a
whole row must list the table's columns in its `CREATE TABLE` order; the build checks that, as two
columns of one type would otherwise swap silently. SQL never contains formatted values. Unit enums
are stored as their serde names, lists as JSON text, timestamps as RFC 3339 text with nanoseconds.
A column holds only a record's key; the table is implied by the column.

A node image's groups (see [`NODE_SETTINGS.md`](NODE_SETTINGS.md#image-definition)) are flat
`node_image` columns. Each kind-only column is `NULL` for the kinds that do not take it, held by a
`CHECK` per column, so a row cannot hold another kind's settings.

## Schema

Tables are `STRICT`. `lab.user` and `node.image` are `ON DELETE RESTRICT`: deleting only the
records of an owner's labs or an image in use would leave VMs, containers and disks running
unmanaged. Everything a lab or node owns cascades from it, so deleting a lab is one `DELETE` that
takes its definition with it, and deleting a user ends their sessions (see
[`SERVER.md`](SERVER.md#sessions-and-identity)).

### Versions

`apply_schema` runs on every start and in `sherpad init`, in one transaction: a failed migration
leaves the database at the version it had. `PRAGMA user_version` is the schema version; each file in
`sql/migrations/` takes it up one, and its number is the version it reaches. A database newer than
the build is refused, so a downgrade never writes a schema it does not understand.
`check_schema_upgrade` makes the same refusals without writing, and the update script runs the new
release's `sherpad check-database` before installing it, so a refused upgrade leaves the running
release in place.

- A released version never changes, `sql/schema/` included. New columns come last, for the mappers.
- SQLite cannot alter most constraints in place, and Turso refuses to drop a table another table
  references, even with deferred foreign keys. So `lab` and `node` are never rebuilt: a column whose
  constraints change is added again under a new name and filled in one `UPDATE`, and a `UNIQUE`
  column that is no longer used is renamed `retired_*` and kept filled.
- A `CHECK` is written with `AND` and `OR`, never as a comparison of two conditions, which Turso
  cannot read back when it re-reads a `STRICT` table's schema, as `VACUUM INTO` does.
- A migration that would drop what a lab still uses refuses to run while such a lab exists, naming
  it (`schema::server_key`).
- Migration 41 renames the built-in `admin` to `sherpa-admin` in place, so its id, password,
  keys, lock state, sessions and labs stay with it. It drops the triggers that protect the account,
  renames the row and makes them again for the new name. It refuses to run while a user named
  `sherpa-admin` exists (`schema::user`), rather than merge two accounts.

### CPU pins

`cpu_pin` holds every host CPU a node is pinned to, one row per CPU and role (`vcpu`, or `thread`
for emulator and I/O threads), and is the only place pins are stored. Rows name the node by its name
in the lab, not its record, because admission claims them during `up`'s validation, before node rows
exist. The rules are triggers, so they hold for any statement:

- `cpu_pin_held` refuses a row whose CPU another node holds when either row is `vcpu`: vCPUs never
  share, QEMU threads may. A node never conflicts with itself, which is how a redeploying node holds
  its old and new pins at once.
- `cpu_pin_unchanged` refuses every `UPDATE`, so no pin can move around the first trigger.
- Deleting a node or its lab deletes its rows.

`claim_cpu_pins` inserts only the rows a node does not hold yet, because the trigger runs before the
primary key check, and re-inserting a row on a CPU that another node also holds from before the
triggers existed would be refused. Overlaps stored before the triggers existed are kept, so the
database still opens, and sherpad warns about each at every start. Pins are held while a lab is
stopped and go only with the lab, or with a failed `up`. Admission's use of them is in
[`SERVER.md`](SERVER.md#cpu-pins-in-admission).

### Sandbox addresses

`sandbox_address` holds the management address each sandbox has of its own, beside its node's:
its IPv4 address, its IPv6 address when the lab has IPv6, and the MAC of its NIC, one row per
sandbox node, keyed on the node's record. `up` writes the row when it gives the node its addresses,
`down`, `resume` and `redeploy` read it, and it goes with the node (`ON DELETE CASCADE`), so a
destroyed or rolled-back lab gives its addresses back. It is a table of its own so that `DbNode`
and the node queries do not carry columns only three models use. The addresses are in
[`SANDBOX.md`](SANDBOX.md#management).

### Talos cluster plans

`talos_cluster_plan` holds what `up` chose for each cluster (endpoint, shared address, load balancer
range), keyed on the lab and the cluster's position in `manifest_talos_cluster`. It is a table of
its own rather than columns of `manifest_talos_cluster`, which holds what the user wrote: the
definition is stored before the plan exists, and every writer of the definition would have to
carry it. Its foreign key cascades, so the plan goes with its cluster. Members' addresses are their
nodes' `mgmt_ipv4`, so the plan does not repeat them.

### Lab definitions

A lab's definition is every parameter of the manifest it was deployed from, as the user wrote it:
once the lab is deployed, the database is the server's record of it, and the server keeps no copy
of the manifest anywhere else. No file contents and no resolved values are stored. The client
resolves `ztp.config` and `$VAR` values before it sends the manifest, so it sends the parts it
changes, as written, beside it (`topology::ManifestAsWritten`). The contents of the files and
scripts a node names are never manifest keys: the client sends them beside the manifest too, by
node name (`topology::NodeFileContents`), and the server puts them into the nodes
(`Manifest::put_file_contents`). The server builds nodes from the resolved manifest and stores
`Manifest::definition(as_written)`. `store_lab_definition` refuses a manifest that still holds
file contents.

Storage is normalised: one-to-one parameters are columns of `lab` and `node`, lists are rows of the
`manifest_*` tables keyed by `position`, and nothing is a JSON or TOML document. Where a node
already records what it was built with, the manifest's value is a `manifest_*` column beside it. A
node's kind is stored only when it is not the model's default, so a manifest that wrote the default
reads back without it. Only empty lists and empty kind groups do not round-trip; they read back as
absent, which every reader treats the same.

`redeploy` stores only its node and the lab-level settings it applies, as it does not rebuild the
lab's links, bridges or clusters. A reader with no definition, or one it cannot read, uses its
defaults and never makes one up (`services::lab_definition::stored_definition`). A web UI redeploy
rebuilds a node from the definition alone, so it refuses one that needs what the definition does
not hold (`validate::check_web_redeployable`).

### Secret values

No secret is stored as plain text: `user.device_password` is sealed, so a database or backup
without its key holds only ciphertext. Lab service credentials (Vault tokens, Keycloak and
SurrealDB admins) are not in the database at all, but in `0600` files in the lab's folder on the
server. `crates/db/src/secret.rs` seals with AES-256-GCM, a fresh nonce per
value, and the value's place (`table.column:row-key`) as associated data, so a value copied to
another row or column does not open.

The key is `/opt/sherpa/.secret/db.key`, beside the JWT secret in a `0700` `sherpa` directory and
never under `/opt/sherpa/db`, so a copy of the database directory does not carry it. `secret_key_check` holds a known value sealed with
the key; sherpad refuses to start on a key that does not open it, or with no key file for a database
that has one, and never writes a new key over a missing one, as nothing sealed would open with it. A
lost key stops the server until the file is restored.

A sealed `TEXT` column holds `SealedText` (`sealed:` and hex), which only `seal_text` makes, and its
triggers refuse any other value, so no writer can store plain text there.

### Device passwords

`user.device_password` is not part of `DbUser`, and only `db::user::device_password` reads it, so
listing or editing users never touches it. Opened, it is a `DevicePassword`, which redacts itself
in `Debug` and has no `Serialize`, so no response can carry it. There is no clear: a user replaces a
forgotten one. Who may set it is in [`SERVER.md`](SERVER.md#device-passwords).

### Login attempts and the lockout

`user.failed_logins` counts the logins started since the account's last successful one, and
`user.locked_at` (RFC 3339 text, `NULL` when not locked) is when the attempt that reached the limit
locked it. Neither is part of `DbUser`: only the lockout's queries (`user/login_claim.sql`,
`user/login_succeeded.sql`, `user/unlock.sql`, `user/list_locked.sql`) touch them, and they leave
`updated_at` alone, so a login does not conflict with an edit of the user. The count is taken with
one `UPDATE … RETURNING` before the password is checked; how the server uses it is in
[`SERVER.md`](SERVER.md#logins-and-the-account-lockout).

`login_attempt` holds one row per attempt on an account that was not locked: `user`, `attempted_at`,
`succeeded`, and `address`, a `BLOB` of the source address's `octets()`, 4 bytes for IPv4 and
16 for IPv6, with an IPv4-mapped IPv6 address stored as its IPv4 address; a `CHECK` holds the
length. It is indexed on `(user, attempted_at)`. Rows are never pruned; they go only with their
user (`ON DELETE CASCADE`).

### Enum values and triggers

Enum columns are checked by the types, not the database: every value is serialized from its enum,
and one a newer release stored fails when its row is read, naming the row. The lifecycle states
(`node.state`, `lab.phase`) decode an unknown value as `unknown` instead, so one row does not fail a
whole listing. Triggers enforce only rules the types cannot express, all as fixed text in
migrations: the built-in `sherpa-admin` cannot be deleted, renamed or demoted; an interface is used by one
link or bridge; and the CPU pin rules above.
