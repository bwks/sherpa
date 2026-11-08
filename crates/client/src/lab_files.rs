//! Lab files the server sends for CLI use, rewritten for this machine and saved in the lab
//! folder's `.sherpa` folder (`shared::util::LabFolder`), which only its owner can enter.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use shared::data::{
    ClusterInfo, DownloadLabResponse, KeycloakInfo, LabInfo, UpResponse, VaultInfo,
};
use shared::konst::{SHERPA_KNOWN_HOSTS_FILE, SHERPA_SSH_PRIVATE_KEY_FILE};
use shared::util::{Emoji, LabFolder, add_lab_ssh_include};

use crate::cluster_credentials::{print_cluster_files, write_cluster_credentials};
use crate::keycloak_credentials::{print_keycloak_files, write_keycloak_credentials};
use crate::private_key::{SSH_PRIVATE_KEY_ERROR_LABEL, ensure_private_dir, write_secret_file};
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

/// Save `files` in `folder`'s `.sherpa` folder, showing each one saved, and return the path of
/// the SSH config. Stops at the first file that cannot be saved.
#[tracing::instrument(level = "debug", skip_all, fields(folder = %folder.root().display()))]
pub(crate) fn save_lab_files(folder: &LabFolder, files: &LabFiles) -> Result<PathBuf> {
    ensure_lab_subdir(folder, &folder.ssh_dir())?;
    let ssh_config = folder.ssh_config();
    fs::write(
        &ssh_config,
        rewrite_ssh_config_for_client(files.ssh_config, folder),
    )
    .with_context(|| format!("Failed to write {}", ssh_config.display()))?;
    println!(
        "{} SSH config created: {}",
        Emoji::Success,
        ssh_config.display()
    );

    if let Some(path) = write_known_hosts(folder, files.known_hosts)? {
        println!(
            "{} Node host keys saved: {}",
            Emoji::Success,
            path.display()
        );
    }
    if !files.ssh_private_key.is_empty() {
        let key = folder.ssh_key();
        write_secret_file(&key, files.ssh_private_key, SSH_PRIVATE_KEY_ERROR_LABEL)?;
        println!(
            "{} SSH private key created: {}",
            Emoji::Success,
            key.display()
        );
    }

    let lab_info = folder.lab_info();
    fs::write(&lab_info, files.lab_info.to_string())
        .with_context(|| format!("Failed to write {}", lab_info.display()))?;
    println!(
        "{} Lab info created: {}",
        Emoji::Success,
        lab_info.display()
    );

    let lab_ca = write_lab_ca(folder, files.lab_ca)?;
    if let Some(path) = &lab_ca {
        println!("{} Lab CA certificate: {}", Emoji::Success, path.display());
    }
    print_vault_files(
        &write_vault_credentials(folder, files.vaults)
            .context("Failed to write Vault credentials")?,
        lab_ca.as_deref(),
    );
    print_keycloak_files(
        &write_keycloak_credentials(folder, files.keycloaks)
            .context("Failed to write Keycloak users")?,
    );
    print_cluster_files(
        &write_cluster_credentials(folder, files.clusters)
            .context("Failed to write cluster credentials")?,
    );
    Ok(ssh_config)
}

/// Make `folder`'s `.sherpa` folder and its sub-folder `dir`, both only for their owner.
/// Refuses a symlinked `.sherpa` or `dir`.
#[tracing::instrument(level = "debug", fields(dir = %dir.display()))]
pub(crate) fn ensure_lab_subdir(folder: &LabFolder, dir: &Path) -> Result<()> {
    ensure_private_dir(&folder.state_dir())?;
    ensure_private_dir(dir)
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
/// - Replaces the server's relative `IdentityFile sherpa_ssh_key` and
///   `UserKnownHostsFile sherpa_known_hosts` with the absolute paths of `folder`'s
///   `.sherpa/ssh/key` and `.sherpa/ssh/known_hosts`, so the config works from anywhere, as
///   through the `~/.ssh/config` include or `ssh -F .sherpa/ssh/config`
/// - On Windows, replaces `UserKnownHostsFile /dev/null` with `UserKnownHostsFile NUL`
pub(crate) fn rewrite_ssh_config_for_client(ssh_config: &str, folder: &LabFolder) -> String {
    let result = ssh_config
        .replace(
            &format!("IdentityFile {SHERPA_SSH_PRIVATE_KEY_FILE}"),
            &format!("IdentityFile {}", folder.ssh_key().display()),
        )
        .replace(
            &format!("UserKnownHostsFile {SHERPA_KNOWN_HOSTS_FILE}"),
            &format!("UserKnownHostsFile {}", folder.known_hosts().display()),
        );

    #[cfg(windows)]
    let result = result.replace("UserKnownHostsFile /dev/null", "UserKnownHostsFile NUL");

    result
}

/// Save the lab's `known_hosts` file in `folder`, and return its path. Nothing is written when
/// the server recorded no host keys. The file is replaced in one step, as nodes being
/// provisioned at the same time read it while another node refreshes it.
#[tracing::instrument(level = "debug", skip_all, fields(folder = %folder.root().display()))]
pub(crate) fn write_known_hosts(folder: &LabFolder, known_hosts: &str) -> Result<Option<PathBuf>> {
    if known_hosts.is_empty() {
        tracing::debug!("The server recorded no node host keys");
        return Ok(None);
    }
    ensure_lab_subdir(folder, &folder.ssh_dir())?;
    let path = folder.known_hosts();
    write_secret_file(&path, known_hosts, "node host keys")?;
    Ok(Some(path))
}

/// Save the lab CA certificate in `folder`, and return its path. Nothing is written when the
/// server sent none (a lab created before the lab CA, or an older server).
#[tracing::instrument(level = "debug", skip_all, fields(folder = %folder.root().display()))]
pub(crate) fn write_lab_ca(folder: &LabFolder, lab_ca: &str) -> Result<Option<PathBuf>> {
    if lab_ca.is_empty() {
        tracing::debug!("The server sent no lab CA certificate");
        return Ok(None);
    }
    ensure_lab_subdir(folder, &folder.ca_dir())?;
    let path = folder.lab_ca();
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

    fn lab_files(lab_info: &LabInfo) -> LabFiles<'_> {
        LabFiles {
            ssh_config: "Host dev dev.fecb1a12\n    HostName 172.31.3.21\n    User sherpa\n    \
                         IdentityFile sherpa_ssh_key\n    StrictHostKeyChecking yes\n    \
                         UserKnownHostsFile sherpa_known_hosts\n",
            ssh_private_key: "private key",
            known_hosts: "172.31.3.21 ssh-ed25519 AAAA\n",
            lab_info,
            lab_ca: "-----BEGIN CERTIFICATE-----\n",
            vaults: &[],
            keycloaks: &[],
            clusters: &[],
        }
    }

    #[test]
    fn the_lab_files_are_saved_in_the_sherpa_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();

        let ssh_config = save_lab_files(&folder, &lab_files(&lab_info)).unwrap();

        assert_eq!(ssh_config, dir.path().join(".sherpa/ssh/config"));
        for path in [
            folder.ssh_key(),
            folder.known_hosts(),
            folder.lab_ca(),
            folder.lab_info(),
        ] {
            assert!(path.is_file(), "{}", path.display());
        }
        let saved: LabInfo = fs::read_to_string(folder.lab_info())
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(saved.id, "fecb1a12");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            names,
            [".sherpa"],
            "nothing is left loose in the lab folder"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_sherpa_folders_are_owner_only_and_secrets_0600() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();

        save_lab_files(&folder, &lab_files(&lab_info)).unwrap();

        for path in [folder.state_dir(), folder.ssh_dir(), folder.ca_dir()] {
            assert_eq!(mode(&path), 0o700, "{}", path.display());
        }
        for path in [folder.ssh_key(), folder.known_hosts()] {
            assert_eq!(mode(&path), 0o600, "{}", path.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_sherpa_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join(".sherpa")).unwrap();
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();

        let error = save_lab_files(&LabFolder::new(dir.path()), &lab_files(&lab_info)).unwrap_err();

        assert!(format!("{error:#}").contains("symlink"), "{error:#}");
        assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_saved_ssh_config_resolves_its_files_in_the_sherpa_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();

        let ssh_config = save_lab_files(&folder, &lab_files(&lab_info)).unwrap();

        let config = jato::SshConfig::load_file(&ssh_config).unwrap();
        let target = config.route("dev.fecb1a12").unwrap().target;
        assert_eq!(target.identity_files, [folder.ssh_key()]);
        assert_eq!(target.known_hosts, [folder.known_hosts()]);
    }

    #[test]
    fn the_lab_ca_is_saved_only_when_the_server_sends_one() {
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());
        assert_eq!(write_lab_ca(&folder, "").unwrap(), None);
        assert!(!folder.state_dir().exists());
        let pem = "-----BEGIN CERTIFICATE-----\nlab\n-----END CERTIFICATE-----\n";
        let path = write_lab_ca(&folder, pem).unwrap().unwrap();
        assert_eq!(path, dir.path().join(".sherpa/ca/lab-ca.crt"));
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
        let folder = LabFolder::new(Path::new("/home/user/lab"));

        let result = rewrite_ssh_config_for_client(server_config, &folder);

        let expected_path = Path::new("/home/user/lab")
            .join(".sherpa")
            .join("ssh")
            .join("key");
        assert!(
            result.contains(&format!("IdentityFile {}", expected_path.display())),
            "Expected absolute path, got: {result}"
        );
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

        let result =
            rewrite_ssh_config_for_client(server_config, &LabFolder::new(Path::new("/tmp/lab")));
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
        let folder = LabFolder::new(Path::new("/home/user/lab"));

        let result = rewrite_ssh_config_for_client(server_config, &folder);

        let count = result
            .matches(&format!("IdentityFile {}", folder.ssh_key().display()))
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
        let folder = LabFolder::new(Path::new("/home/user/lab"));

        let result = rewrite_ssh_config_for_client(server_config, &folder);

        assert!(
            result.contains(&format!(
                "UserKnownHostsFile {}\n",
                folder.known_hosts().display()
            )),
            "{result}"
        );
        assert!(!result.contains("UserKnownHostsFile sherpa_known_hosts"));
        #[cfg(unix)]
        assert!(result.contains("UserKnownHostsFile /dev/null"));
    }

    #[test]
    fn known_hosts_is_written_only_when_keys_were_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());
        assert_eq!(write_known_hosts(&folder, "").unwrap(), None);
        assert!(!folder.known_hosts().exists());

        let line = "172.31.1.11 ssh-ed25519 AAAAC3\n";
        let path = write_known_hosts(&folder, line).unwrap().unwrap();
        assert_eq!(path, dir.path().join(".sherpa/ssh/known_hosts"));
        assert_eq!(fs::read_to_string(path).unwrap(), line);
    }
}
