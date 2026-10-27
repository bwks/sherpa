//! Connecting to a lab node, asking about a server whose host key is not recorded, as
//! OpenSSH does with `StrictHostKeyChecking ask`.
//!
//! Nothing is asked inside the SSH handshake. A strict connection is refused with the key;
//! once the user confirms its fingerprint, the key is recorded in the first `known_hosts`
//! file and a second strict connection accepts exactly that key and no other.

use std::io::{self, Write};

use anyhow::{Context, Result, bail};
use jato::{ConnectOptions, Error, KnownHosts, Node, Route, Session};
use tokio::task;

const YES: &str = "yes";

/// Connect along `route`, asking before recording a server's unknown host key. A changed or
/// revoked key is refused.
#[tracing::instrument(skip_all, level = "debug")]
pub async fn connect(route: &Route, options: &ConnectOptions) -> Result<Session> {
    let (node, fingerprint, key, known_hosts) = match route.connect(options).await {
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
    route.connect(options).await.context("Failed to connect")
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
