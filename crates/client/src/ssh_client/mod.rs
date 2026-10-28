//! Connecting to lab nodes through the Sherpa server with the Jato SSH library, for
//! `sherpa ssh` and the SSH provisioners: which hops may ask on the terminal, which failures
//! are worth retrying, and fetching node host keys from the server when one does not match.
mod connect;
mod host_keys;

pub use connect::{PromptScope, connect, retryable};
pub use host_keys::HostKeyRefresh;
