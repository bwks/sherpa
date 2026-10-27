//! An interactive shell on a lab node for `sherpa ssh`, through the Jato SSH library: the
//! local terminal, the `~.` escape, and the server host key prompt.
mod escape;
mod host_key;
mod prompter;
mod session;
mod terminal;

pub use host_key::connect;
pub use session::run_shell;
