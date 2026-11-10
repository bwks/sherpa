//! Login sessions: one row per issued token.
//!
//! The rules for when a session may be used, and which events end it, are in
//! `sherpad::auth::session`. This module only stores the rows. A user's sessions are also
//! deleted by `delete_user` (the foreign key cascades) and by `change_user_password`.

mod create;
mod delete;
mod read;

pub use create::create_session;
pub use delete::{delete_expired_sessions, delete_session};
pub use read::find_session;
