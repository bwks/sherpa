//! Connecting to a lab node through the Sherpa server with Jato, asking on the terminal as
//! OpenSSH does. Jato asks: about a server whose host key is not recorded (the lab config
//! leaves `StrictHostKeyChecking` at `ask` for the server), and for a key passphrase or
//! password, with answers kept for the run in a `PromptCache`.

use std::sync::Arc;

use anyhow::{Context, Result};
use jato::{ConnectOptions, Error, Node, PromptCache, Route, Session, TerminalPrompter};

/// Which hops may ask for a passphrase or password.
#[derive(Clone, Copy, Debug)]
pub enum PromptScope {
    /// Every hop, as `ssh` does: for `sherpa ssh`.
    AllHops,
    /// Only the jump hosts. A node whose login is not ready yet must fail and be retried,
    /// not ask for a password: for provisioners.
    JumpHostsOnly,
}

/// Connect along `route`, asking on the terminal about a server's unknown host key and for a
/// key passphrase or password the hops in `scope` need, with answers kept in `cache`. A
/// changed or revoked key is refused.
#[tracing::instrument(skip_all, level = "debug")]
pub async fn connect(
    route: &Route,
    options: &ConnectOptions,
    scope: PromptScope,
    cache: &PromptCache,
) -> Result<Session> {
    let prompter = cache.prompter(Arc::new(TerminalPrompter::new()));
    let mut jumps = Vec::with_capacity(route.jumps.len());
    for jump in &route.jumps {
        jumps.push(jump.hop().prompter(Arc::clone(&prompter)));
    }
    let target = match scope {
        PromptScope::AllHops => route.target.hop().prompter(prompter),
        PromptScope::JumpHostsOnly => route.target.hop(),
    };
    let options = route.target.connect_options(options);
    Session::connect_via(&jumps, &target, &options)
        .await
        .context("Failed to connect")
}

/// Whether a failure to reach `target` may pass if tried again: the node is still booting,
/// not accepting connections or its login yet. A host key problem, the Sherpa server
/// refusing the login, or a declined question will not.
pub fn retryable(error: &anyhow::Error, target: &Node) -> bool {
    match error.downcast_ref::<Error>() {
        Some(Error::AuthFailed { node, .. }) => node == target,
        Some(
            Error::HostKeyUnknown { .. }
            | Error::HostKeyChanged { .. }
            | Error::HostKeyRevoked { .. },
        ) => false,
        Some(_) => true,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::path::PathBuf;

    fn node() -> Node {
        Node::new("172.31.1.21", 22)
    }

    fn server() -> Node {
        Node::new("10.100.58.10", 22)
    }

    fn auth_failed(node: Node) -> anyhow::Error {
        anyhow::Error::new(Error::AuthFailed {
            node,
            username: "sherpa".into(),
            tried: Vec::new(),
        })
        .context("Failed to connect")
    }

    #[test]
    fn the_node_not_accepting_the_login_yet_is_retried() {
        assert!(retryable(&auth_failed(node()), &node()));
        let refused = anyhow::Error::new(Error::Connect {
            node: node(),
            source: io::Error::from(io::ErrorKind::ConnectionRefused),
        });
        assert!(retryable(&refused, &node()));
    }

    #[test]
    fn the_server_refusing_the_login_is_not_retried() {
        assert!(!retryable(&auth_failed(server()), &node()));
    }

    #[test]
    fn host_key_problems_and_other_errors_are_not_retried() {
        let changed = anyhow::Error::new(Error::HostKeyChanged {
            node: server(),
            fingerprint: "SHA256:abc".into(),
            known_hosts: PathBuf::from("/home/me/.ssh/known_hosts"),
            line: 3,
        });
        assert!(!retryable(&changed, &node()));
        assert!(!retryable(
            &anyhow::anyhow!("Host key verification failed"),
            &node()
        ));
    }
}
