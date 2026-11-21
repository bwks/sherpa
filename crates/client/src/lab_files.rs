//! Lab files the server sends for CLI use, rewritten for this machine and saved in the lab
//! folder's `.sherpa` folder (`shared::util::LabFolder`), which only its owner can enter.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use shared::data::{
    ClusterInfo, DownloadLabResponse, KeycloakInfo, LabInfo, SurrealDbInfo, UpResponse, VaultInfo,
};
use shared::konst::SHERPA_KNOWN_HOSTS_FILE;
use shared::util::{Emoji, LabFolder, add_lab_ssh_include};

use crate::cluster_credentials::{print_cluster_files, write_cluster_credentials};
use crate::keycloak_credentials::{print_keycloak_files, write_keycloak_credentials};
use crate::secret_file::{ensure_private_dir, write_secret_file};
use crate::surreal_db_credentials::{print_surreal_db_files, write_surreal_db_credentials};
use crate::vault_credentials::{print_vault_files, write_vault_credentials};

/// The lab files the server sends with `sherpa up` and `sherpa download`.
pub(crate) struct LabFiles<'a> {
    pub ssh_config: &'a str,
    pub known_hosts: &'a str,
    pub lab_info: &'a LabInfo,
    pub lab_ca: &'a str,
    pub vaults: &'a [VaultInfo],
    pub keycloaks: &'a [KeycloakInfo],
    pub surreal_dbs: &'a [SurrealDbInfo],
    pub clusters: &'a [ClusterInfo],
}

impl<'a> From<&'a UpResponse> for LabFiles<'a> {
    fn from(response: &'a UpResponse) -> Self {
        Self {
            ssh_config: &response.ssh_config,
            known_hosts: &response.known_hosts,
            lab_info: &response.lab_info,
            lab_ca: &response.lab_ca,
            vaults: &response.vaults,
            keycloaks: &response.keycloaks,
            surreal_dbs: &response.surreal_dbs,
            clusters: &response.clusters,
        }
    }
}

impl<'a> From<&'a DownloadLabResponse> for LabFiles<'a> {
    fn from(response: &'a DownloadLabResponse) -> Self {
        Self {
            ssh_config: &response.ssh_config,
            known_hosts: &response.known_hosts,
            lab_info: &response.lab_info,
            lab_ca: &response.lab_ca,
            vaults: &response.vaults,
            keycloaks: &response.keycloaks,
            surreal_dbs: &response.surreal_dbs,
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
            .context("Failed to write Keycloak credentials")?,
    );
    print_surreal_db_files(
        &write_surreal_db_credentials(folder, files.surreal_dbs)
            .context("Failed to write SurrealDB credentials")?,
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
/// - Replaces the server's relative `UserKnownHostsFile sherpa_known_hosts` with the absolute
///   path of `folder`'s `.sherpa/ssh/known_hosts`, so the config works from anywhere, as
///   through the `~/.ssh/config` include or `ssh -F .sherpa/ssh/config`. The config names no
///   key file: nodes take the user's own keys.
/// - On Windows, replaces `UserKnownHostsFile /dev/null` with `UserKnownHostsFile NUL`
pub(crate) fn rewrite_ssh_config_for_client(ssh_config: &str, folder: &LabFolder) -> String {
    let result = ssh_config.replace(
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
"#;

    fn lab_files(lab_info: &LabInfo) -> LabFiles<'_> {
        LabFiles {
            ssh_config: "Host dev dev.fecb1a12\n    HostName 172.31.3.21\n    User dev\n    \
                         StrictHostKeyChecking yes\n    \
                         UserKnownHostsFile sherpa_known_hosts\n",
            known_hosts: "172.31.3.21 ssh-ed25519 AAAA\n",
            lab_info,
            lab_ca: "-----BEGIN CERTIFICATE-----\n",
            vaults: &[],
            keycloaks: &[],
            surreal_dbs: &[],
            clusters: &[],
        }
    }

    #[test]
    fn saved_lab_files_002_the_files_are_saved_at_their_paths_under_sherpa() {
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();

        let ssh_config = save_lab_files(&folder, &lab_files(&lab_info)).unwrap();

        assert_eq!(ssh_config, dir.path().join(".sherpa/ssh/config"));
        for path in [
            ssh_config,
            dir.path().join(".sherpa/ssh/known_hosts"),
            dir.path().join(".sherpa/ca/lab-ca.crt"),
            dir.path().join(".sherpa/lab-info.toml"),
        ] {
            assert!(path.is_file(), "{}", path.display());
        }
        let saved: LabInfo = fs::read_to_string(dir.path().join(".sherpa/lab-info.toml"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            (saved.id.as_str(), saved.name.as_str(), saved.user.as_str()),
            ("fecb1a12", "sherpa-dev", "dev")
        );
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

    #[test]
    fn saved_lab_files_003_the_lab_ca_is_saved_only_when_the_server_sends_one() {
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());
        assert_eq!(write_lab_ca(&folder, "").unwrap(), None);
        assert!(!folder.state_dir().exists());
        let pem = "-----BEGIN CERTIFICATE-----\nlab\n-----END CERTIFICATE-----\n";
        let path = write_lab_ca(&folder, pem).unwrap().unwrap();
        assert_eq!(path, dir.path().join(".sherpa/ca/lab-ca.crt"));
        assert_eq!(fs::read_to_string(path).unwrap(), pem);
    }

    #[cfg(unix)]
    #[test]
    fn lab_file_privacy_002_saving_leaves_the_folders_700_and_known_hosts_600() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();

        save_lab_files(&folder, &lab_files(&lab_info)).unwrap();

        for path in [folder.state_dir(), folder.ssh_dir(), folder.ca_dir()] {
            assert_eq!(mode(&path), 0o700, "{}", path.display());
        }
        assert_eq!(mode(&folder.known_hosts()), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn lab_file_privacy_004_a_symlinked_sherpa_or_folder_of_it_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();
        let open_folder = || {
            let elsewhere = tempfile::tempdir().unwrap();
            fs::set_permissions(elsewhere.path(), fs::Permissions::from_mode(0o755)).unwrap();
            elsewhere
        };

        let dir = tempfile::tempdir().unwrap();
        let elsewhere = open_folder();
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join(".sherpa")).unwrap();
        let error = save_lab_files(&LabFolder::new(dir.path()), &lab_files(&lab_info)).unwrap_err();
        assert!(format!("{error:#}").contains("symlink"), "{error:#}");
        assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
        assert_eq!(mode(elsewhere.path()), 0o755);

        let dir = tempfile::tempdir().unwrap();
        let elsewhere = open_folder();
        fs::create_dir(dir.path().join(".sherpa")).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join(".sherpa/ssh")).unwrap();
        let error = save_lab_files(&LabFolder::new(dir.path()), &lab_files(&lab_info)).unwrap_err();
        assert!(format!("{error:#}").contains("symlink"), "{error:#}");
        assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
        assert_eq!(mode(elsewhere.path()), 0o755);
    }

    #[test]
    fn ssh_config_hosts_007_the_known_hosts_file_is_named_by_its_full_path() {
        let server_config = "\
Host 172.31.1.11 dev01.abc123
    HostName 172.31.1.11
    Port 22
    User alice
    StrictHostKeyChecking yes
    UserKnownHostsFile sherpa_known_hosts
Host 172.31.1.12 sw01.abc123
    HostName 172.31.1.12
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null";
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());

        let result = rewrite_ssh_config_for_client(server_config, &folder);

        let known_hosts = format!("    UserKnownHostsFile {}", folder.known_hosts().display());
        let expected: Vec<&str> = server_config
            .lines()
            .map(|line| {
                if line.ends_with("UserKnownHostsFile sherpa_known_hosts") {
                    known_hosts.as_str()
                } else {
                    line
                }
            })
            .collect();
        let lines: Vec<&str> = result.lines().collect();
        #[cfg(unix)]
        assert_eq!(lines, expected);
        #[cfg(not(unix))]
        assert_eq!(lines[..lines.len() - 1], expected[..expected.len() - 1]);
    }

    #[test]
    fn node_host_keys_008_known_hosts_is_saved_only_when_keys_were_recorded() {
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
