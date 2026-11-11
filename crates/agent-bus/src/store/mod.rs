//! The host's record of every message on the bus: an embedded Turso database that keeps the
//! messages, and the tasks, runs and models derived from them, for as long as the host keeps
//! the file. `crate::record` opens it, for one command at a time.
//!
//! - `database`: opening the file and migrating its schema.
//! - `sql`: the statements, one file each under `sql/`.
//! - `task`: how a message changes its task's row, without I/O.
//! - `write`: recording one message, idempotently, in one transaction.
//! - `read`: what the commands read.

mod cells;
mod database;
pub mod read;
mod sql;
mod task;
mod write;

pub use database::{FILE, Store};
pub use read::HistoryRow;
pub use task::{Recorded, TaskState};
pub use write::record;
