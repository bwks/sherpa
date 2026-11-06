//! Lab files the server sends for CLI use, rewritten for this machine and saved in the lab
//! folder.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use shared::data::{
    ClusterInfo, DownloadLabResponse, KeycloakInfo, LabInfo, UpResponse, VaultInfo,
};
use shared::konst::{
    LAB_FILE_NAME, SHERPA_KNOWN_HOSTS_FILE, SHERPA_LAB_CA_FILE, SHERPA_SSH_CONFIG_FILE,
    SHERPA_SSH_PRIVATE_KEY_FILE,
};
use shared::util::{Emoji, add_lab_ssh_include};

use crate::cluster_credentials::{print_cluster_files, write_cluster_credentials};
use crate::keycloak_credentials::{print_keycloak_files, write_keycloak_credentials};
use crate::private_key::{SSH_PRIVATE_KEY_ERROR_LABEL, write_secret_file};
use crate::vault_credentials::{print_vault_files, write_vault_credentials};

/// The lab files the server sends with `sherpa up` and `sherpa download`.
pub(crate) struct LabFiles<'a> {
    pub ssh_config: &'a str,
    pub ssh_private_key: &'a str,
    pub known_hosts: &'a str,
    pub lab_info: &'a LabInfo,
    pub lab_ca: &'a str,
    pub vaults: &'a [VaultInfo],
    pub keycloaks: &'a [KeycloakInfo],
    pub clusters: &'a [ClusterInfo],
}

impl<'a> From<&'a UpResponse> for LabFiles<'a> {
    fn from(response: &'a UpResponse) -> Self {
        Self {
            ssh_config: &response.ssh_config,
            ssh_private_key: &response.ssh_private_key,
            known_hosts: &response.known_hosts,
            lab_info: &response.lab_info,
            lab_ca: &response.lab_ca,
            vaults: &response.vaults,
            keycloaks: &response.keycloaks,
            clusters: &response.clusters,
        }
    }
}

impl<'a> From<&'a DownloadLabResponse> for LabFiles<'a> {
    fn from(response: &'a DownloadLabResponse) -> Self {
        Self {
            ssh_config: &response.ssh_config,
            ssh_private_key: &response.ssh_private_key,
            known_hosts: &response.known_hosts,
            lab_info: &response.lab_info,
            lab_ca: &response.lab_ca,
            vaults: &response.vaults,
            keycloaks: &response.keycloaks,
            clusters: &response.clusters,
        }
    }
}

/// Save `files` in the lab folder `cwd`, showing each one saved, and return the path of the SSH
/// config. Stops at the first file that cannot be saved.
#[tracing::instrument(level = "debug", skip_all, fields(%cwd))]
pub(crate) fn save_lab_files(cwd: &str, files: &LabFiles) -> Result<PathBuf> {
    let dir = Path::new(cwd);
    let ssh_config = dir.join(SHERPA_SSH_CONFIG_FILE);
    fs::write(
        &ssh_config,
        rewrite_ssh_config_for_client(files.ssh_config, cwd),
    )
    .with_context(|| format!("Failed to write {}", ssh_config.display()))?;
    println!(
        "{} SSH config created: {}",
        Emoji::Success,
        ssh_config.display()
    );

    if let Some(path) = write_known_hosts(cwd, files.known_hosts)? {
        println!(
            "{} Node host keys saved: {}",
            Emoji::Success,
            path.display()
        );
    }
    if !files.ssh_private_key.is_empty() {
        let key = dir.join(SHERPA_SSH_PRIVATE_KEY_FILE);
        write_secret_file(&key, files.ssh_private_key, SSH_PRIVATE_KEY_ERROR_LABEL)?;
        println!(
            "{} SSH private key created: {}",
            Emoji::Success,
            key.display()
        );
    }

    let lab_info = dir.join(LAB_FILE_NAME);
    fs::write(&lab_info, files.lab_info.to_string())
        .with_context(|| format!("Failed to write {}", lab_info.display()))?;
    println!(
        "{} Lab info created: {}",
        Emoji::Success,
        lab_info.display()
    );

    let lab_ca = write_lab_ca(cwd, files.lab_ca)?;
    if let Some(path) = &lab_ca {
        println!("{} Lab CA certificate: {}", Emoji::Success, path.display());
    }
    print_vault_files(
        &write_vault_credentials(dir, files.vaults).context("Failed to write Vault credentials")?,
        lab_ca.as_deref(),
    );
    print_keycloak_files(
        &write_keycloak_credentials(dir, files.keycloaks)
            .context("Failed to write Keycloak users")?,
    );
    print_cluster_files(
        &write_cluster_credentials(dir, files.clusters)
            .context("Failed to write cluster credentials")?,
    );
    Ok(ssh_config)
}

/// Register the lab's SSH config in `~/.ssh/sherpa_lab_hosts`, so its nodes can be reached by
/// name from anywhere. A failure only warns: the config still works with `ssh -F`.
pub(crate) fn register_ssh_config(ssh_config: &Path) {
    match add_lab_ssh_include(&ssh_config.to_string_lossy()) {
        Ok(()) => println!(
            "{} SSH config registered in ~/.ssh/sherpa_lab_hosts",
            Emoji::Success
        ),
        Err(e) => println!(
            "\n{} Warning: Failed to register SSH config in ~/.ssh/sherpa_lab_hosts: {e:#}",
            Emoji::Warning
        ),
    }
}

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
/// the server recorded no host keys. The file is replaced in one step, as nodes being
/// provisioned at the same time read it while another node refreshes it.
#[tracing::instrument(level = "debug", skip_all, fields(%cwd))]
pub(crate) fn write_known_hosts(cwd: &str, known_hosts: &str) -> Result<Option<PathBuf>> {
    if known_hosts.is_empty() {
        tracing::debug!("The server recorded no node host keys");
        return Ok(None);
    }
    let path = Path::new(cwd).join(SHERPA_KNOWN_HOSTS_FILE);
    write_secret_file(&path, known_hosts, "node host keys")?;
    Ok(Some(path))
}

/// Save the lab CA certificate in `cwd`, and return its path. Nothing is written when the
/// server sent none (a lab created before the lab CA, or an older server).
#[tracing::instrument(level = "debug", skip_all, fields(%cwd))]
pub(crate) fn write_lab_ca(cwd: &str, lab_ca: &str) -> Result<Option<PathBuf>> {
    if lab_ca.is_empty() {
        tracing::debug!("The server sent no lab CA certificate");
        return Ok(None);
    }
    let path = Path::new(cwd).join(SHERPA_LAB_CA_FILE);
    fs::write(&path, lab_ca).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAB_INFO: &str = r#"id = "fecb1a12"
name = "sherpa-dev"
user = "dev"
ipv4_network = "172.31.3.0/24"
ipv4_gateway = "172.31.3.1"
ipv4_router = "172.31.3.2"
loopback_network = "127.127.3.0/24"
"#;

    #[test]
    fn the_lab_files_are_saved_in_the_lab_folder() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();
        let files = LabFiles {
            ssh_config: "Host dev\n    IdentityFile sherpa_ssh_key\n",
            ssh_private_key: "private key",
            known_hosts: "172.31.3.21 ssh-ed25519 AAAA\n",
            lab_info: &lab_info,
            lab_ca: "-----BEGIN CERTIFICATE-----\n",
            vaults: &[],
            keycloaks: &[],
            clusters: &[],
        };

        let ssh_config = save_lab_files(cwd, &files).unwrap();

        assert_eq!(ssh_config, dir.path().join(SHERPA_SSH_CONFIG_FILE));
        assert!(fs::read_to_string(&ssh_config).unwrap().contains(&format!(
            "IdentityFile {}",
            dir.path().join(SHERPA_SSH_PRIVATE_KEY_FILE).display()
        )));
        for name in [
            SHERPA_SSH_PRIVATE_KEY_FILE,
            SHERPA_KNOWN_HOSTS_FILE,
            SHERPA_LAB_CA_FILE,
        ] {
            assert!(dir.path().join(name).is_file(), "{name}");
        }
        let saved: LabInfo = fs::read_to_string(dir.path().join(LAB_FILE_NAME))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(saved.id, "fecb1a12");
    }

    #[test]
    fn the_lab_ca_is_saved_only_when_the_server_sends_one() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        assert_eq!(write_lab_ca(cwd, "").unwrap(), None);
        let pem = "-----BEGIN CERTIFICATE-----\nlab\n-----END CERTIFICATE-----\n";
        let path = write_lab_ca(cwd, pem).unwrap().unwrap();
        assert_eq!(path, dir.path().join("lab-ca.crt"));
        assert_eq!(fs::read_to_string(path).unwrap(), pem);
    }

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
            result.contains(&format!("IdentityFile {expected_path}")),
            "Expected absolute path, got: {result}"
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
            .matches(&format!("IdentityFile {expected_path}"))
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
