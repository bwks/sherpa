//! Login attempts and the account lockout's count.
//!
//! The rules, the limit and what is logged are in `sherpad::auth::login`. This module keeps the
//! count on `user` and the `login_attempt` rows, each change in one statement or transaction.

mod claim;
mod read;

pub use claim::{LoginClaim, claim_login, finish_login};
pub use read::list_login_attempts;
