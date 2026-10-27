//! Connecting to lab nodes through the Sherpa server with the Jato SSH library, for
//! `sherpa ssh` and the SSH provisioners: which hops may ask on the terminal, and which
//! failures are worth retrying.
mod connect;

pub use connect::{PromptScope, connect, retryable};
