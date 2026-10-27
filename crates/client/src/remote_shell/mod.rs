//! An interactive shell on a lab node for `sherpa ssh`: the local terminal and the `~.`
//! escape. Connecting is `ssh_client`'s.
mod escape;
mod session;
mod terminal;

pub use session::run_shell;
