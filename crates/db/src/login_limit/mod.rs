//! The login limit's failed logins: one row per failure, by username, address and time.
//!
//! The rules, which limit refuses which login, are in `sherpad::auth::login_limit`. This module
//! stores the rows and counts them. Starting an attempt counts and writes in one transaction under
//! the database's write lock, so logins made at the same moment cannot get past a limit. No
//! password, nor anything made from one, is stored.

mod create;
mod delete;
mod read;
mod types;

pub use create::begin_login_attempt;
pub use delete::{clear_login_failures, delete_expired_login_failures, delete_login_failure};
pub use read::count_login_failures;
pub use types::{
    LoginAddress, LoginAdmission, LoginFailureId, LoginKeys, LoginSubject, LoginWindow,
};
