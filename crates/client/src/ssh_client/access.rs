//! How one command reaches a lab's nodes over SSH.

use std::path::Path;

use anyhow::Result;

use shared::data::ClientConfig;

use crate::ssh_client::host_keys::HostKeyRefresh;
use crate::ssh_client::jump::SshJump;

/// How a command reaches a lab's nodes: directly, or through `jump`, the Sherpa server, with
/// `--ssh-jump`; and where it fetches a node's host key again when it does not match.
#[derive(Clone, Debug)]
pub struct SshAccess {
    pub refresh: HostKeyRefresh,
    pub jump: Option<SshJump>,
}

impl SshAccess {
    /// Access to the nodes of lab `lab_id`, whose folder is `lab_dir`, on the server at
    /// `server_url`: through that server as the logged-in user when `ssh_jump` is set,
    /// otherwise directly.
    ///
    /// # Errors
    ///
    /// Fails when `ssh_jump` is set and there is no login, or `server_url` names no host.
    pub fn new(
        lab_id: &str,
        server_url: &str,
        config: &ClientConfig,
        lab_dir: &Path,
        ssh_jump: bool,
    ) -> Result<Self> {
        let jump = if ssh_jump {
            Some(SshJump::logged_in(server_url)?)
        } else {
            None
        };
        Ok(Self {
            refresh: HostKeyRefresh::new(lab_id, server_url, config, lab_dir),
            jump,
        })
    }
}
