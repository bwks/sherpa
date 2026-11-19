//! The Sherpa server as an SSH jump host. A lab's SSH config reaches every node directly;
//! `--ssh-jump` adds the jump for one command, as `ssh -J` adds one to a config that has none.
//! Nothing about it is saved.

use std::fmt;

use anyhow::{Context, Result, bail};
use jato::{Route, SshConfig};
use url::{Host, Url};

use crate::token::{load_token, username_from_token};

/// The jump through the Sherpa server: the logged-in Sherpa user at the server's host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshJump {
    user: String,
    host: String,
}

impl SshJump {
    /// The logged-in Sherpa user at the host of `server_url`, the server's RPC WebSocket.
    ///
    /// # Errors
    ///
    /// Fails when there is no login, or `server_url` names no host.
    pub fn logged_in(server_url: &str) -> Result<Self> {
        let token = load_token().context("Authentication required: run sherpa login")?;
        Self::new(&username_from_token(&token)?, server_url)
    }

    /// `user` at the host of `server_url`, an address as SSH takes it: an IPv6 address
    /// without its URL brackets.
    fn new(user: &str, server_url: &str) -> Result<Self> {
        let url =
            Url::parse(server_url).with_context(|| format!("Invalid server URL: {server_url}"))?;
        let host = match url.host() {
            Some(Host::Domain(name)) => name.to_owned(),
            Some(Host::Ipv4(address)) => address.to_string(),
            Some(Host::Ipv6(address)) => address.to_string(),
            None => bail!("The server URL {server_url} names no host to jump through"),
        };
        Ok(Self {
            user: user.to_owned(),
            host,
        })
    }

    /// `route` through this jump, in place of any jump it had, as `ssh -J` does. The server's
    /// own settings, such as its host key check, are read from `config` as for any host.
    ///
    /// # Errors
    ///
    /// Fails when a setting in `config` that applies to the server has a bad value.
    pub fn apply(&self, config: &SshConfig, route: &mut Route) -> Result<()> {
        let mut jump = config
            .host(&self.host)
            .with_context(|| format!("Cannot read the jump host {self} from the SSH config"))?;
        jump.user.clone_from(&self.user);
        route.jumps = vec![jump];
        Ok(())
    }
}

impl fmt::Display for SshJump {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.user, self.host)
    }
}

/// The way to `host` in the lab SSH config `config`: straight to the node, or through `jump`.
///
/// # Errors
///
/// Fails when a setting for `host` or the jump host has a bad value.
pub fn node_route(config: &SshConfig, host: &str, jump: Option<&SshJump>) -> Result<Route> {
    let mut route = config
        .route(host)
        .with_context(|| format!("Cannot read {host} from the lab SSH config"))?;
    if let Some(jump) = jump {
        jump.apply(config, &mut route)?;
    }
    Ok(route)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const LAB_SSH_CONFIG: &str = "Host 172.31.0.11 web01.ab12cd34\n    HostName 172.31.0.11\n    \
                                  User alice\n    StrictHostKeyChecking no\n";

    fn lab_ssh_config() -> SshConfig {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        fs::write(&path, LAB_SSH_CONFIG).unwrap();
        SshConfig::load_file(&path).unwrap()
    }

    #[test]
    fn ssh_access_007_the_jump_is_the_logged_in_user_at_the_servers_host() {
        let config = lab_ssh_config();
        for (server_url, server) in [
            ("wss://10.100.58.10:3030/ws", "10.100.58.10"),
            ("wss://sherpa.example:3030/ws", "sherpa.example"),
            ("ws://[2001:db8::10]:3030/ws", "2001:db8::10"),
        ] {
            let jump = SshJump::new("bob", server_url).unwrap();
            let route = node_route(&config, "web01.ab12cd34", Some(&jump)).unwrap();
            assert_eq!(route.jumps.len(), 1, "{server_url}");
            assert_eq!(route.jumps[0].hostname, server, "{server_url}");
            assert_eq!(route.jumps[0].port, 22, "{server_url}");
            assert_eq!(route.jumps[0].user, "bob", "{server_url}");
            assert_eq!(route.target.hostname, "172.31.0.11");
            assert_eq!(route.target.user, "alice");
        }
    }
}
