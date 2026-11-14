//! The host's record, read by the commands behind the `record` feature: `usage`, `show`,
//! `history`, `open`, `link` and `work`, and the tables they print.
//!
//! - `command`: the commands, and opening the record caught up from the bus.
//! - `usage`: the table of `usage`.
//! - `grouped`: `usage --by`, the tasks added up by agent, model or week.
//! - `history`: the table of `history`.
//! - `work`: what `work show`, `open`, `link` and `work backfill` print.

mod command;
mod grouped;
mod history;
pub mod usage;
mod work;

pub use command::{Command, run};
