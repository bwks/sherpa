//! The host's record, read by the commands behind the `record` feature: `usage`, `show`,
//! `history`, `open`, `link`, `work` and `host`, and the tables they print.
//!
//! - `command`: the commands, and opening the record caught up from the bus.
//! - `usage`: the table of `usage`.
//! - `grouped`: `usage --by`, the tasks added up by agent, model or week.
//! - `history`: the table of `history`.
//! - `work`: what `work show`, `open`, `link` and `work backfill` print.
//! - `host`: `host import` and `host status`, the `SessionEnd` hook, and the import every
//!   record command runs.

mod command;
mod grouped;
mod history;
mod host;
pub mod usage;
mod work;

pub use command::{Command, refresh_after_send, run};
