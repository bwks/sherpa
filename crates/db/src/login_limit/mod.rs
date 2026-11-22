//! The login limit's counts: one row per username and address pair, per address and per username
//! that has attempts counted against it, each kind in a table of its own.
//!
//! The rules, which limit locks which key and for how long, are in `sherpad::auth::login_limit`.
//! This module stores the rows, and runs each step that reads and then changes counts in one
//! transaction under the database's write lock, so logins made at the same moment cannot count
//! past a limit. No password, nor anything made from one, is stored.

mod create;
mod delete;
mod key;
mod read;
mod types;
mod update;

pub use create::begin_login_attempt;
pub use delete::delete_expired_login_limits;
pub use read::count_login_limits;
pub(crate) use types::LoginCount;
pub use types::{LoginAddress, LoginExpiry, LoginKeys, LoginRelease, LoginSubject};
pub use update::{give_back_login_attempt, lock_full_login_keys};
