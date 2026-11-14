//! Connecting to lab nodes, and to the Sherpa server on the way, with the Jato SSH library,
//! for `sherpa ssh`, `sherpa console` and the SSH provisioners: which hops may ask on the
//! terminal, which failures are worth retrying, and fetching node host keys from the server
//! when one does not match.
mod connect;
mod host_keys;

pub use connect::{PromptScope, connect, connect_server, retryable};
pub use host_keys::HostKeyRefresh;
