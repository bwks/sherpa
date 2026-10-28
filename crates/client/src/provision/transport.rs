//! SSH transport for provisioners, through the Jato SSH library: connecting to a node through
//! the Sherpa server as the lab's `sherpa_ssh_config` says, running commands and copying
//! files over SFTP. No `ssh` or `scp` program is run.
//!
//! Only the jump to the Sherpa server may ask for a passphrase or password, and answers are
//! kept for the run, so provisioning many nodes asks once. A node is never asked about: while
//! it boots its login fails, and `wait_for_node` retries instead.
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use jato::{ConnectOptions, Event, Exit, Node, PromptCache, Route, Session, SshConfig};

use shared::konst::{
    PROVISIONER_COPY_TIMEOUT, PROVISIONER_SSH_CONNECT_TIMEOUT, SSH_PROMPT_AUTH_TIMEOUT,
};

use crate::ssh_client::{self, HostKeyRefresh, PromptScope};

/// The mode a copied file is written with; `finish_file_command` then sets the final mode.
const COPY_MODE: u32 = 0o600;

/// A node reached through the lab's SSH config.
pub struct SshTarget {
    node_name: String,
    host: String,
    route: Route,
    prompts: PromptCache,
    refresh: HostKeyRefresh,
}

/// An open connection to a node.
pub struct NodeSession {
    host: String,
    session: Session,
}

impl SshTarget {
    /// The node `node_name`, by its `<node>.<lab-id>` alias in `config`. Answers to prompts
    /// are kept in `prompts`, shared by every node of a run, and a node host key that does not
    /// match is fetched again with `refresh`.
    ///
    /// Every node in a lab SSH config is reached through the Sherpa server. An alias with no
    /// `Host` block resolves, as in `ssh`, to a direct connection to the alias as a hostname,
    /// which would only fail its DNS lookup until the ready timeout, so it fails here.
    pub fn new(
        config: &SshConfig,
        node_name: &str,
        lab_id: &str,
        prompts: &PromptCache,
        refresh: &HostKeyRefresh,
    ) -> Result<Self> {
        let host = format!("{node_name}.{lab_id}");
        let route = config
            .route(&host)
            .with_context(|| format!("Cannot read {host} from the lab SSH config"))?;
        if route.jumps.is_empty() {
            bail!(
                "{host} is not in the lab SSH config; was the node added to the manifest after \
                 the lab was created?"
            );
        }
        Ok(Self {
            node_name: node_name.to_owned(),
            host,
            route,
            prompts: prompts.clone(),
            refresh: refresh.clone(),
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// The node itself, the last hop.
    pub fn node(&self) -> Node {
        self.route.target.node()
    }

    /// Connect and log in to the node.
    #[tracing::instrument(skip(self), fields(host = %self.host), level = "debug")]
    pub async fn connect(&self) -> Result<NodeSession> {
        let mut options = ConnectOptions::default();
        options.connect_timeout = Duration::from_secs(PROVISIONER_SSH_CONNECT_TIMEOUT);
        options.auth_timeout = Duration::from_secs(SSH_PROMPT_AUTH_TIMEOUT);
        let session = ssh_client::connect(
            &self.route,
            &options,
            PromptScope::JumpHostsOnly,
            &self.prompts,
            &self.node_name,
            &self.refresh,
        )
        .await
        .with_context(|| format!("Cannot connect to {}", self.host))?;
        Ok(NodeSession {
            host: self.host.clone(),
            session,
        })
    }
}

impl NodeSession {
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Run `command` on the node, keeping its error output to report a failure.
    pub async fn run(&self, command: &str) -> Result<()> {
        self.execute(command, None).await
    }

    /// Run `command` on the node with `input` on its stdin, showing its output as it runs.
    pub async fn run_with_input(&self, command: &str, input: &[u8]) -> Result<()> {
        self.execute(command, Some(input)).await
    }

    /// Copy the client file `src` to `dst` on the node, relative to the login user's home
    /// unless absolute, over SFTP as OpenSSH's `scp` does. The data goes to a private temporary
    /// file that is renamed over `dst`, so it is never readable by others.
    #[tracing::instrument(skip(self), fields(host = %self.host), level = "debug")]
    pub async fn copy_to(&self, src: &Path, dst: &str) -> Result<()> {
        let data = fs::read(src).with_context(|| format!("Cannot read {}", src.display()))?;
        let failed = || format!("Copying {} to {}:{dst} failed", src.display(), self.host);
        let mut sftp = self
            .session
            .sftp(Duration::from_secs(PROVISIONER_COPY_TIMEOUT))
            .await
            .with_context(failed)?;
        let uploaded = sftp
            .upload(dst, &data, COPY_MODE)
            .await
            .with_context(failed);
        if let Err(error) = sftp.close().await {
            tracing::debug!(host = %self.host, %error, "Failed to close the SFTP session cleanly");
        }
        uploaded
    }

    /// Tell the node and the server the session is ending.
    pub async fn close(self) {
        if let Err(error) = self.session.close().await {
            tracing::debug!(host = %self.host, %error, "Failed to close the SSH session cleanly");
        }
    }

    /// Run `command`, sending `input` (or nothing) to its stdin. With input, which is how
    /// scripts run, the output is shown as it arrives; otherwise only the error output is kept.
    #[tracing::instrument(skip(self, command, input), fields(host = %self.host), level = "debug")]
    async fn execute(&self, command: &str, input: Option<&[u8]>) -> Result<()> {
        let mut exec = self
            .session
            .exec(command)
            .await
            .with_context(|| format!("Cannot run a command on {}", self.host))?;
        if let Some(input) = input {
            exec.write_stdin(input)
                .await
                .with_context(|| format!("Cannot send input to {}", self.host))?;
        }
        exec.close_stdin()
            .await
            .with_context(|| format!("Cannot send input to {}", self.host))?;
        let show = input.is_some();
        let mut errors = Vec::new();
        let mut exit = None;
        while let Some(event) = exec
            .next_event()
            .await
            .with_context(|| format!("Lost the command on {}", self.host))?
        {
            match event {
                Event::Stdout(bytes) => {
                    if show {
                        write_through(&mut io::stdout(), &bytes)?;
                    }
                }
                Event::Stderr(bytes) => {
                    if show {
                        write_through(&mut io::stderr(), &bytes)?;
                    } else {
                        errors.extend(bytes);
                    }
                }
                Event::Exit(status) => exit = Some(status),
                other => tracing::debug!(?other, "Command event not shown"),
            }
        }
        let exit = exit.unwrap_or(Exit::Unknown);
        if exit == Exit::Code(0) {
            return Ok(());
        }
        let errors = String::from_utf8_lossy(&errors);
        if errors.trim().is_empty() {
            bail!("{exit} on {}", self.host);
        }
        bail!("{exit} on {}: {}", self.host, errors.trim())
    }
}

fn write_through(out: &mut impl Write, bytes: &[u8]) -> Result<()> {
    out.write_all(bytes)
        .context("Cannot show the command's output")?;
    out.flush().context("Cannot show the command's output")
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::ClientConfig;

    const LAB_SSH_CONFIG: &str = "Host web01.ab12cd34\n    HostName 172.31.0.11\n    \
                                  User sherpa\n    ProxyJump admin@sherpa.example:22\n";

    fn lab_ssh_config() -> SshConfig {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sherpa_ssh_config");
        fs::write(&path, LAB_SSH_CONFIG).unwrap();
        SshConfig::load_file(&path).unwrap()
    }

    fn refresh() -> HostKeyRefresh {
        HostKeyRefresh::new(
            "ab12cd34",
            "wss://sherpa.example:3030/ws",
            &ClientConfig::default(),
            Path::new("/nonexistent"),
        )
    }

    #[test]
    fn a_node_in_the_lab_ssh_config_is_reached_through_the_server() {
        let target = SshTarget::new(
            &lab_ssh_config(),
            "web01",
            "ab12cd34",
            &PromptCache::new(),
            &refresh(),
        )
        .unwrap();
        assert_eq!(target.host(), "web01.ab12cd34");
        assert_eq!(target.node().host(), "172.31.0.11");
    }

    #[test]
    fn a_node_missing_from_the_lab_ssh_config_fails_at_once() {
        let error = SshTarget::new(
            &lab_ssh_config(),
            "web02",
            "ab12cd34",
            &PromptCache::new(),
            &refresh(),
        )
        .err()
        .unwrap();
        assert_eq!(
            error.to_string(),
            "web02.ab12cd34 is not in the lab SSH config; was the node added to the manifest \
             after the lab was created?"
        );
    }
}
