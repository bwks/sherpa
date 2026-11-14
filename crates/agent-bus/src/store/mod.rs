//! The host's record of every message on the bus: an embedded Turso database that keeps the
//! messages, and the tasks, runs and models derived from them, for as long as the host keeps
//! the file. `crate::record` opens it, for one command at a time.
//!
//! - `kind`: what the record reads of a message's kind.
//! - `database`: opening the file and migrating its schema.
//! - `sql`: the statements, one file each under `sql/`.
//! - `task`: how a message changes its task's row, without I/O.
//! - `facts`: the refs a message names, and how they were learned, without I/O.
//! - `write`: recording one message, idempotently, in one transaction.
//! - `work`: work items: applying a message's facts, `link` and `work backfill`.
//! - `read`: what the commands read.
//! - `items`: what the work commands read: items, open tasks and runs by model.
//! - `host`: the host's usage and time: the import's rows, attribution and what reports read.

mod cells;
mod database;
mod facts;
pub mod host;
pub mod items;
mod kind;
pub mod read;
mod sql;
mod task;
mod work;
mod write;

pub use database::{FILE, Store};
pub use facts::Ref;
pub use read::HistoryRow;
pub use task::{Recorded, TaskState};
pub use work::{Backfilled, Linked, backfill, item_for, link, ref_item};
pub use write::record;
