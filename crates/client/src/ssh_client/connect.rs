//! Connecting to a lab node with Jato, directly or through the Sherpa server as a jump host,
//! asking on the terminal as OpenSSH does. Jato asks: about a server whose host key is not
//! recorded (the lab config leaves `StrictHostKeyChecking` at `ask` for the server), and for a
//! key passphrase or password, with answers kept for the run in a `PromptCache`. Lab nodes
//! whose host key the server recorded are checked against the lab's `known_hosts` file without
//! asking; when a node's key is unknown or has changed, the current keys are fetched from the
//! server, with a warning for a changed key, and the connection is tried once more.
//!
//! A node reached directly that has no route from this machine fails at once, with a hint to
//! use `--ssh-jump` or the lab's tailnet, rather than being retried until a ready timeout.

use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use jato::{
    ConnectOptions, Error, HostConfig, Node, PromptCache, Prompter, Route, Secret, Session,
    TerminalPrompter,
};
use tokio::net::TcpStream;
use tokio::time;

use shared::konst::SSH_DIRECT_ROUTE_TIMEOUT;

use crate::error::ClientError;
use crate::ssh_client::host_keys::HostKeyRefresh;

/// Why a lab node's host key was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyMismatch {
    Changed,
    Unknown,
}

/// Which questions the hops may ask.
#[derive(Clone, Copy, Debug)]
pub enum PromptScope {
    AllHops,
    NoNodePassword,
}

/// Asks the inner prompter for key passphrases only: a password is not asked for, and an
/// unknown host key is refused.
#[derive(Debug)]
struct PassphraseOnly(Arc<dyn Prompter>);

impl Prompter for PassphraseOnly {
    fn passphrase(&self, path: &Path) -> Option<Secret> {
        self.0.passphrase(path)
    }

    fn password(&self, node: &Node, username: &str) -> Option<Secret> {
        tracing::debug!(%node, username, "not asking for a node password");
        None
    }
}

/// Connect along `route` to the lab node `node_name`, asking on the terminal about a
/// server's unknown host key and for a key passphrase or password as `scope` allows, with
/// answers kept in `cache`. A node reached directly is checked first for a route from this
/// machine, and fails at once without one. When the node's host key is unknown or has
/// changed, the lab's keys are fetched with `refresh` and the connection tried once more; a
/// changed key is warned about. A revoked key, or the server's changed key, is refused.
#[tracing::instrument(skip(route, options, cache, refresh), level = "debug")]
pub async fn connect(
    route: &Route,
    options: &ConnectOptions,
    scope: PromptScope,
    cache: &PromptCache,
    node_name: &str,
    refresh: &HostKeyRefresh,
) -> Result<Session> {
    if route.jumps.is_empty() {
        require_direct_route(&route.target.node(), node_name).await?;
    }
    let error = match attempt(&route.jumps, &route.target, options, scope, cache).await {
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
            eprintln!("Loaded the SSH host key of {node_name} from the Sherpa server.");
        }
    }
    match attempt(&route.jumps, &route.target, options, scope, cache).await {
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

/// One connection attempt to `target` through `jumps`.
async fn attempt(
    jumps: &[HostConfig],
    target: &HostConfig,
    options: &ConnectOptions,
    scope: PromptScope,
    cache: &PromptCache,
) -> Result<Session, Error> {
    let prompter = cache.prompter(Arc::new(TerminalPrompter::new()));
    let mut hops = Vec::with_capacity(jumps.len());
    for jump in jumps {
        hops.push(jump.hop().prompter(Arc::clone(&prompter)));
    }
    let target_hop = target.hop().prompter(node_prompter(scope, prompter));
    let options = target.connect_options(options);
    Session::connect_via(&hops, &target_hop, &options).await
}

/// What the node, the last hop, may ask through `prompter` in `scope`.
fn node_prompter(scope: PromptScope, prompter: Arc<dyn Prompter>) -> Arc<dyn Prompter> {
    match scope {
        PromptScope::AllHops => prompter,
        PromptScope::NoNodePassword => Arc::new(PassphraseOnly(prompter)),
    }
}

/// Stop unless `node`, the lab node `node_name` reached with no jump host, has a route from
/// this machine: a TCP connection to its SSH port is answered, or refused by the node itself,
/// within `SSH_DIRECT_ROUTE_TIMEOUT`. The error is not a Jato error, so it is never retried.
#[tracing::instrument(level = "debug")]
async fn require_direct_route(node: &Node, node_name: &str) -> Result<()> {
    let limit = Duration::from_secs(SSH_DIRECT_ROUTE_TIMEOUT);
    let cause = match time::timeout(limit, TcpStream::connect((node.host(), node.port()))).await {
        Ok(Ok(_)) => return Ok(()),
        Ok(Err(error)) if error.kind() == ErrorKind::ConnectionRefused => {
            tracing::debug!(%node, "node refused the connection; it is reachable");
            return Ok(());
        }
        Ok(Err(error)) => error.to_string(),
        Err(_) => format!("no answer within {}s", limit.as_secs()),
    };
    Err(ClientError::NoRouteToNode {
        node_name: node_name.to_owned(),
        address: node.to_string(),
        cause,
    }
    .into())
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
/// not accepting connections or its login yet, or dropped the connection while starting. A
/// host key problem, the Sherpa server refusing the login, a local file or SSH config that
/// cannot be read, or a declined question will not.
pub fn retryable(error: &anyhow::Error, target: &Node) -> bool {
    let Some(error) = error.downcast_ref::<Error>() else {
        return false;
    };
    match error {
        Error::AuthFailed { node, .. } => node == target,
        Error::Connect { .. }
        | Error::Tunnel { .. }
        | Error::Timeout { .. }
        | Error::ConnectionLost { .. }
        | Error::Protocol { .. } => true,
        _ => false,
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
    fn provision_run_009_a_node_refusing_the_login_or_connection_is_tried_again() {
        assert!(retryable(&auth_failed(node()), &node()));
        let refused = anyhow::Error::new(Error::Connect {
            node: node(),
            source: io::Error::from(io::ErrorKind::ConnectionRefused),
        });
        assert!(retryable(&refused, &node()));
    }

    #[test]
    fn provision_run_010_the_jump_server_refusing_the_login_is_not_tried_again() {
        assert!(!retryable(&auth_failed(server()), &node()));
    }

    fn changed(node: Node) -> Error {
        Error::HostKeyChanged {
            node,
            fingerprint: "SHA256:abc".into(),
            known_hosts: PathBuf::from("/home/me/lab/.sherpa/ssh/known_hosts"),
            line: 1,
        }
    }

    fn unknown(node: Node) -> Error {
        Error::HostKeyUnknown {
            node,
            fingerprint: "SHA256:abc".into(),
            key: "ssh-ed25519 AAAAC3".into(),
            known_hosts: vec![PathBuf::from("/home/me/lab/.sherpa/ssh/known_hosts")],
        }
    }

    #[test]
    fn node_host_keys_009_a_node_s_unknown_or_changed_key_makes_the_cli_fetch_keys() {
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
    fn node_host_keys_010_the_server_s_key_and_other_failures_fetch_nothing() {
        assert_eq!(key_mismatch(&changed(server()), &node()), None);
        assert_eq!(key_mismatch(&unknown(server()), &node()), None);
        let refused = Error::Connect {
            node: node(),
            source: io::Error::from(io::ErrorKind::ConnectionRefused),
        };
        assert_eq!(key_mismatch(&refused, &node()), None);
    }

    /// A prompter that answers every question.
    #[derive(Debug)]
    struct Answers;

    impl Prompter for Answers {
        fn passphrase(&self, _path: &Path) -> Option<Secret> {
            Some(Secret::new("key-passphrase".into()))
        }

        fn password(&self, _node: &Node, _username: &str) -> Option<Secret> {
            Some(Secret::new("node-password".into()))
        }

        fn confirm_host_key(&self, _node: &Node, _fingerprint: &str) -> bool {
            true
        }
    }

    fn answer(secret: Option<Secret>) -> Option<String> {
        secret.map(|secret| secret.expose().to_owned())
    }

    #[test]
    fn ssh_access_008_a_provisioner_asks_the_node_for_a_key_passphrase_but_never_a_password() {
        let prompter = node_prompter(PromptScope::NoNodePassword, Arc::new(Answers));
        let key = Path::new("/home/me/.ssh/id_ed25519");
        assert_eq!(
            answer(prompter.passphrase(key)).as_deref(),
            Some("key-passphrase")
        );
        assert_eq!(answer(prompter.password(&node(), "alice")), None);
    }
}
