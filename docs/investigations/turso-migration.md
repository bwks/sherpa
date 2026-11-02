# Moving the database to embedded Turso

> Status: phase 0 spike passed (2026-11-01); migration done in #415. How the database works now,
> including the design that followed from this spike, is in
> [`../architecture/DATABASE.md`](../architecture/DATABASE.md).

## Goal

Replace the SurrealDB container with Turso, a Rust rewrite of SQLite, embedded in `sherpad`. The
plan and the decisions on it are in #415.

## Spike

A scratch program outside the workspace exercised `turso` 0.8.3 with `default-features = false`,
on the dev server (Ubuntu 24.04, 14 vCPUs), with a schema shaped like Sherpa's.

| Check | Result |
|---|---|
| `ON DELETE CASCADE` across lab → node → link and node → bridge member | Pass |
| `ON DELETE RESTRICT`, and inserting a row whose parent does not exist | Pass: both refused with `Constraint("FOREIGN KEY constraint failed")` |
| Multi-column `UNIQUE`, and a nullable `UNIQUE` column holding several NULLs | Pass: `UNIQUE constraint failed: lab.(name, user)` |
| `CHECK` constraints for enum values and ranges, `STRICT` column types | Pass: the error names the failing expression |
| `BEFORE DELETE` and `BEFORE UPDATE` triggers with `RAISE(ABORT, ...)` | Pass: the message reaches Rust unchanged, as `Error(...)` |
| Transactions: rollback, and a transaction dropped without commit | Pass: both leave no rows |
| `RETURNING`, upsert, JSON functions, `ALTER TABLE ADD COLUMN`, `PRAGMA user_version` | Pass |
| `SIGKILL` of a process in the middle of writes, then reopen | Pass: committed rows kept, `integrity_check` ok |
| `VACUUM INTO` while another task writes | Pass: the copy opens and passes `integrity_check` |
| `sqlite3` CLI on the file after Turso closes it | Pass |
| `sqlite3` CLI on the file while Turso has it open | **Fails**: `database is locked`. Turso holds the file exclusively; multi-process access is experimental |
| One `Connection` used by several tasks at once | **Fails**: `Misuse("concurrent use forbidden")` |
| 64 tasks, a connection each, 2,560 write transactions, deferred or `BEGIN IMMEDIATE` | **0.4–0.5% fail** with `Busy("database is locked")`, despite a 5 s busy timeout |
| The same with an in-process write lock (`tokio::sync::Mutex`) | Pass: no errors, 2,560 transactions in 8.2 s, the same rate as without the lock |
| `busy_timeout` with one writer holding the lock for 0.2, 1, 3 and 7 s | Waits the full time and succeeds; at 7 s fails after 5 s, as configured |

The busy timeout works. Under 64-way contention some writers wait longer than the timeout,
because the busy wait does not serve waiters in order. A tokio mutex is fair, so it removes the
failures.

The `turso` crate's default features install mimalloc as the process's global allocator. Sherpa
turns default features off.

## Consequences for the design

These now live in [`../architecture/DATABASE.md`](../architecture/DATABASE.md), which is
authoritative; the list below records what the spike decided.

- **Connections:** each operation takes its own connection from the `Database`. A connection is
  never shared between tasks.
- **Writes:** every write and every transaction holds one `Database`-wide write lock. Reads take
  no lock.
- **Outside access:** while `sherpad` runs, nothing else can open the file. Backups go through
  `sherpad` with `VACUUM INTO`, and the importer and any manual inspection run with `sherpad`
  stopped.
