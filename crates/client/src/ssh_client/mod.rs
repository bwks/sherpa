//! Connecting to lab nodes with the Jato SSH library, for `sherpa ssh` and the SSH
//! provisioners: directly, or through the Sherpa server with `--ssh-jump`; which hops may ask
//! on the terminal, which failures are worth retrying, and fetching node host keys from the
//! server when one does not match.
mod access;
mod connect;
mod host_keys;
mod jump;

pub use access::SshAccess;
pub use connect::{PromptScope, connect, retryable};
pub use host_keys::HostKeyRefresh;
pub use jump::node_route;
