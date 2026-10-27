//! Connecting to a lab node through the Sherpa server with Jato, asking as OpenSSH does: about
//! a server whose host key is not recorded (`StrictHostKeyChecking ask`), and for a key
//! passphrase or password.
//!
//! Nothing is asked inside the SSH handshake. A strict connection is refused with the key;
//! once the user confirms its fingerprint, the key is recorded in the first `known_hosts`
//! file and a second strict connection accepts exactly that key and no other.

use std::io::{self, Write};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use jato::{ConnectOptions, Error, KnownHosts, Node, Prompter, Route, Session};
use tokio::task;

use super::prompter::PromptCache;

const YES: &str = "yes";

/// Which hops may ask for a passphrase or password.
#[derive(Clone, Copy, Debug)]
pub enum PromptScope {
    /// Every hop, as `ssh` does: for `sherpa ssh`.
    AllHops,
    /// Only the jump hosts. A node whose login is not ready yet must fail and be retried,
    /// not ask for a password: for provisioners.
    JumpHostsOnly,
}

/// Connect along `route`, asking before recording a server's unknown host key, and asking
/// on the terminal for a key passphrase or password the hops in `scope` need, with answers
/// kept in `cache`. A changed or revoked key is refused.
#[tracing::instrument(skip_all, level = "debug")]
pub async fn connect(
    route: &Route,
    options: &ConnectOptions,
    scope: PromptScope,
    cache: &PromptCache,
) -> Result<Session> {
    let first = connect_once(route, options, scope, &cache.terminal()).await;
    let (node, fingerprint, key, known_hosts) = match first {
        Ok(session) => return Ok(session),
        Err(Error::HostKeyUnknown {
            node,
            fingerprint,
            key,
            known_hosts,
            ..
        }) => (node, fingerprint, key, known_hosts),
        Err(error) => return Err(error).context("Failed to connect"),
    };
    let question = unknown_host_question(&node, &fingerprint);
    let answer = task::spawn_blocking(move || ask(&question))
        .await
        .context("Failed to ask about the host key")?
        .context("Failed to read the answer")?;
    if !confirms(&answer, &fingerprint) {
        bail!("Host key verification failed for {node}");
    }
    let Some(file) = known_hosts.first() else {
        bail!("No known_hosts file to record the host key of {node} in");
    };
    KnownHosts::new(file)
        .record(&node, &key)
        .await
        .with_context(|| format!("Failed to record the host key of {node}"))?;
    eprintln!(
        "Warning: Permanently added '{node}' to the list of known hosts ({}).",
        file.display()
    );
    connect_once(route, options, scope, &cache.terminal())
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

/// One connection along `route`, with `prompter` on the hops `scope` names.
async fn connect_once(
    route: &Route,
    options: &ConnectOptions,
    scope: PromptScope,
    prompter: &Arc<dyn Prompter>,
) -> Result<Session, Error> {
    let mut jumps = Vec::with_capacity(route.jumps.len());
    for jump in &route.jumps {
        jumps.push(jump.hop().prompter(Arc::clone(prompter)));
    }
    let target = match scope {
        PromptScope::AllHops => route.target.hop().prompter(Arc::clone(prompter)),
        PromptScope::JumpHostsOnly => route.target.hop(),
    };
    let options = route.target.connect_options(options);
    Session::connect_via(&jumps, &target, &options).await
}

/// The question OpenSSH asks about an unknown server.
fn unknown_host_question(node: &Node, fingerprint: &str) -> String {
    format!(
        "The authenticity of host '{node}' can't be established.\n\
         Its key fingerprint is {fingerprint}.\n\
         Are you sure you want to continue connecting (yes/no/[fingerprint])? "
    )
}

/// Whether `answer` accepts the key: `yes`, or the fingerprint itself.
fn confirms(answer: &str, fingerprint: &str) -> bool {
    let answer = answer.trim();
    answer.eq_ignore_ascii_case(YES) || answer == fingerprint
}

/// Print `question` and read one line from the terminal.
fn ask(question: &str) -> io::Result<String> {
    let mut stderr = io::stderr();
    stderr.write_all(question.as_bytes())?;
    stderr.flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer)
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

    #[test]
    fn the_question_names_the_server_and_fingerprint() {
        let node = Node::new("10.100.58.10", 22);
        let question = unknown_host_question(&node, "SHA256:abc");
        assert!(question.contains("10.100.58.10:22"), "{question}");
        assert!(question.contains("SHA256:abc"), "{question}");
        assert!(question.ends_with("(yes/no/[fingerprint])? "), "{question}");
    }

    #[test]
    fn yes_or_the_fingerprint_confirms() {
        assert!(confirms("yes\n", "SHA256:abc"));
        assert!(confirms("  YES \r\n", "SHA256:abc"));
        assert!(confirms("SHA256:abc\n", "SHA256:abc"));
    }

    #[test]
    fn anything_else_refuses() {
        for answer in ["no\n", "y\n", "\n", "SHA256:abd\n", "yes please\n"] {
            assert!(!confirms(answer, "SHA256:abc"), "{answer:?}");
        }
    }
}
