//! Connecting to a lab node through the Sherpa server with Jato, asking on the terminal as
//! OpenSSH does. Jato asks: about a server whose host key is not recorded (the lab config
//! leaves `StrictHostKeyChecking` at `ask` for the server), and for a key passphrase or
//! password, with answers kept for the run in a `PromptCache`. Lab nodes whose host key the
//! server recorded are checked against the lab's `known_hosts` file without asking; when a
//! node's key is unknown or has changed, the current keys are fetched from the server, with a
//! warning for a changed key, and the connection is tried once more.

use std::sync::Arc;

use anyhow::{Context, Result};
use jato::{ConnectOptions, Error, Node, PromptCache, Route, Session, TerminalPrompter};

use crate::ssh_client::host_keys::HostKeyRefresh;

/// Why a lab node's host key was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyMismatch {
    /// The lab's `known_hosts` file records another key for the node.
    Changed,
    /// The file records no key for the node.
    Unknown,
}

/// Which hops may ask for a passphrase or password.
#[derive(Clone, Copy, Debug)]
pub enum PromptScope {
    /// Every hop, as `ssh` does: for `sherpa ssh`.
    AllHops,
    /// Only the jump hosts. A node whose login is not ready yet must fail and be retried,
    /// not ask for a password: for provisioners.
    JumpHostsOnly,
}

/// Connect along `route` to the lab node `node_name`, asking on the terminal about a
/// server's unknown host key and for a key passphrase or password the hops in `scope` need,
/// with answers kept in `cache`. When the node's host key is unknown or has changed, the
/// lab's keys are fetched with `refresh` and the connection tried once more; a changed key
/// is warned about. A revoked key, or the server's changed key, is refused.
#[tracing::instrument(skip(route, options, cache, refresh), level = "debug")]
pub async fn connect(
    route: &Route,
    options: &ConnectOptions,
    scope: PromptScope,
    cache: &PromptCache,
    node_name: &str,
    refresh: &HostKeyRefresh,
) -> Result<Session> {
    let error = match attempt(route, options, scope, cache).await {
        Ok(session) => return Ok(session),
        Err(error) => error,
    };
    let Some(mismatch) = key_mismatch(&error, &route.target.node()) else {
        return Err(error).context("Failed to connect");
    };
    if let Err(refresh_error) = refresh.refresh(node_name).await {
        return Err(error).context(format!(
            "Failed to connect, and fetching the node's host key from the Sherpa server failed: \
             {refresh_error:#}"
        ));
    }
    match mismatch {
        KeyMismatch::Changed => eprintln!(
            "Warning: the SSH host key of {node_name} has changed. Loaded its new key from the \
             Sherpa server."
        ),
        KeyMismatch::Unknown => {
            eprintln!("Loaded the SSH host key of {node_name} from the Sherpa server.")
        }
    }
    match attempt(route, options, scope, cache).await {
        Ok(session) => Ok(session),
        Err(error) => {
            let context = match key_mismatch(&error, &route.target.node()) {
                Some(_) => {
                    "Failed to connect: the node's host key does not match the key the \
                            Sherpa server has for it"
                }
                None => "Failed to connect",
            };
            Err(error).context(context)
        }
    }
}

/// One connection attempt along `route`.
async fn attempt(
    route: &Route,
    options: &ConnectOptions,
    scope: PromptScope,
    cache: &PromptCache,
) -> Result<Session, Error> {
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
    Session::connect_via(&jumps, &target, &options).await
}

/// Whether `error` refused the host key of `target`, the lab node, rather than of a jump host.
fn key_mismatch(error: &Error, target: &Node) -> Option<KeyMismatch> {
    match error {
        Error::HostKeyChanged { node, .. } if node == target => Some(KeyMismatch::Changed),
        Error::HostKeyUnknown { node, .. } if node == target => Some(KeyMismatch::Unknown),
        _ => None,
    }
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

    fn changed(node: Node) -> Error {
        Error::HostKeyChanged {
            node,
            fingerprint: "SHA256:abc".into(),
            known_hosts: PathBuf::from("/home/me/lab/sherpa_known_hosts"),
            line: 1,
        }
    }

    fn unknown(node: Node) -> Error {
        Error::HostKeyUnknown {
            node,
            fingerprint: "SHA256:abc".into(),
            key: "ssh-ed25519 AAAAC3".into(),
            known_hosts: vec![PathBuf::from("/home/me/lab/sherpa_known_hosts")],
        }
    }

    #[test]
    fn the_nodes_unknown_or_changed_key_is_a_mismatch() {
        assert_eq!(
            key_mismatch(&changed(node()), &node()),
            Some(KeyMismatch::Changed)
        );
        assert_eq!(
            key_mismatch(&unknown(node()), &node()),
            Some(KeyMismatch::Unknown)
        );
    }

    #[test]
    fn the_servers_key_and_other_errors_are_not_a_node_mismatch() {
        assert_eq!(key_mismatch(&changed(server()), &node()), None);
        assert_eq!(key_mismatch(&unknown(server()), &node()), None);
        let refused = Error::Connect {
            node: node(),
            source: io::Error::from(io::ErrorKind::ConnectionRefused),
        };
        assert_eq!(key_mismatch(&refused, &node()), None);
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
