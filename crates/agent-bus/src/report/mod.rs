//! The host's record, read by the commands behind the `record` feature: `usage`, `show` and
//! `history`, and the tables they print.
//!
//! - `command`: the commands, and opening the record caught up from the bus.
//! - `usage`: the table of `usage`.
//! - `history`: the table of `history`.

mod command;
mod history;
pub mod usage;

pub use command::{Command, run};
