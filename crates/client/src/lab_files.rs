//! Lab files the server sends for CLI use, rewritten for this machine and saved in the lab
//! folder.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use shared::konst::{SHERPA_KNOWN_HOSTS_FILE, SHERPA_SSH_PRIVATE_KEY_FILE};

/// Rewrite server-generated SSH config for client-side use.
///
/// - Replaces the relative `IdentityFile sherpa_ssh_key` and
///   `UserKnownHostsFile sherpa_known_hosts` with absolute paths in the working directory,
///   so the config works from anywhere, as through the `~/.ssh/config` include
/// - On Windows, replaces `UserKnownHostsFile /dev/null` with `UserKnownHostsFile NUL`
pub(crate) fn rewrite_ssh_config_for_client(ssh_config: &str, cwd: &str) -> String {
    let abs_key_path = Path::new(cwd)
        .join(SHERPA_SSH_PRIVATE_KEY_FILE)
        .to_string_lossy()
        .to_string();
    let abs_known_hosts_path = Path::new(cwd)
        .join(SHERPA_KNOWN_HOSTS_FILE)
        .to_string_lossy()
        .to_string();

    let result = ssh_config
        .replace(
            &format!("IdentityFile {SHERPA_SSH_PRIVATE_KEY_FILE}"),
            &format!("IdentityFile {abs_key_path}"),
        )
        .replace(
            &format!("UserKnownHostsFile {SHERPA_KNOWN_HOSTS_FILE}"),
            &format!("UserKnownHostsFile {abs_known_hosts_path}"),
        );

    #[cfg(windows)]
    let result = result.replace("UserKnownHostsFile /dev/null", "UserKnownHostsFile NUL");

    result
}

/// Save the lab's `known_hosts` file in `cwd`, and return its path. Nothing is written when
/// the server recorded no host keys.
pub(crate) fn write_known_hosts(cwd: &str, known_hosts: &str) -> Result<Option<PathBuf>> {
    if known_hosts.is_empty() {
        tracing::debug!("The server recorded no node host keys");
        return Ok(None);
    }
    let path = Path::new(cwd).join(SHERPA_KNOWN_HOSTS_FILE);
    fs::write(&path, known_hosts).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_ssh_config_absolute_identity_file() {
        let server_config = "\
Host 172.31.1.11 dev01.abc123 dev01.abc123.sherpa.lab.local
    HostName 172.31.1.11
    Port 22
    User sherpa
    IdentityFile sherpa_ssh_key
    UserKnownHostsFile /dev/null";

        let result = rewrite_ssh_config_for_client(server_config, "/home/user/lab");

        // IdentityFile should use platform-native path joining
        let expected_path = Path::new("/home/user/lab")
            .join(SHERPA_SSH_PRIVATE_KEY_FILE)
            .to_string_lossy()
            .to_string();
        assert!(
            result.contains(&format!("IdentityFile {}", expected_path)),
            "Expected absolute path, got: {}",
            result
        );
        // Original relative path should be gone
        assert!(!result.contains("IdentityFile sherpa_ssh_key\n"));
    }

    #[test]
    fn test_rewrite_ssh_config_preserves_other_content() {
        let server_config = "\
Host 172.31.1.11 dev01.abc123
    HostName 172.31.1.11
    Port 22
    User sherpa
    IdentityFile sherpa_ssh_key
    StrictHostKeyChecking no";

        let result = rewrite_ssh_config_for_client(server_config, "/tmp/lab");
        assert!(result.contains("HostName 172.31.1.11"));
        assert!(result.contains("Port 22"));
        assert!(result.contains("User sherpa"));
        assert!(result.contains("StrictHostKeyChecking no"));
    }

    #[test]
    fn test_rewrite_ssh_config_multiple_hosts() {
        let server_config = "\
Host 172.31.1.11 dev01.abc123
    IdentityFile sherpa_ssh_key
Host 172.31.1.12 dev02.abc123
    IdentityFile sherpa_ssh_key";

        let result = rewrite_ssh_config_for_client(server_config, "/home/user/lab");

        let expected_path = Path::new("/home/user/lab")
            .join(SHERPA_SSH_PRIVATE_KEY_FILE)
            .to_string_lossy()
            .to_string();
        let count = result
            .matches(&format!("IdentityFile {}", expected_path))
            .count();
        assert_eq!(
            count, 2,
            "Both hosts should have absolute IdentityFile paths"
        );
    }

    #[test]
    fn the_known_hosts_file_gets_an_absolute_path() {
        let server_config = "\
Host 172.31.1.11 dev01.abc123
    StrictHostKeyChecking yes
    UserKnownHostsFile sherpa_known_hosts
Host 172.31.1.12 sw01.abc123
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null";

        let result = rewrite_ssh_config_for_client(server_config, "/home/user/lab");

        let expected_path = Path::new("/home/user/lab")
            .join(SHERPA_KNOWN_HOSTS_FILE)
            .to_string_lossy()
            .to_string();
        assert!(
            result.contains(&format!("UserKnownHostsFile {expected_path}\n")),
            "{result}"
        );
        assert!(!result.contains("UserKnownHostsFile sherpa_known_hosts"));
        #[cfg(unix)]
        assert!(result.contains("UserKnownHostsFile /dev/null"));
    }

    #[test]
    fn known_hosts_is_written_only_when_keys_were_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_string_lossy().to_string();
        assert_eq!(write_known_hosts(&cwd, "").unwrap(), None);
        assert!(!dir.path().join(SHERPA_KNOWN_HOSTS_FILE).exists());

        let line = "172.31.1.11 ssh-ed25519 AAAAC3\n";
        let path = write_known_hosts(&cwd, line).unwrap().unwrap();
        assert_eq!(path, dir.path().join(SHERPA_KNOWN_HOSTS_FILE));
        assert_eq!(fs::read_to_string(path).unwrap(), line);
    }
}
