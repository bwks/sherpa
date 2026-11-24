//! Lab files the server sends for CLI use, rewritten for this machine and saved in the lab
//! folder's `.sherpa` folder (`shared::util::LabFolder`), which only its owner can enter.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use shared::data::{
    ClusterInfo, DownloadLabResponse, KeycloakInfo, LabInfo, SurrealDbInfo, UpResponse, VaultInfo,
};
use shared::konst::SHERPA_KNOWN_HOSTS_FILE;
use shared::util::{Emoji, LabFolder, add_lab_ssh_include, ssh_config_path};

use crate::cluster_credentials::{ClusterFiles, cluster_credential_files, print_cluster_files};
use crate::keycloak_credentials::{KeycloakFile, keycloak_credential_files, print_keycloak_files};
use crate::secret_file::{ensure_private_dir, write_secret_file};
use crate::surreal_db_credentials::{
    SurrealDbFile, print_surreal_db_files, surreal_db_credential_files,
};
use crate::vault_credentials::{VaultFile, print_vault_files, vault_credential_files};

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

/// A file of a lab that `save_lab_files` writes in the lab folder.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LabFile {
    pub path: PathBuf,
    pub content: String,
    pub secret: Option<&'static str>,
}

/// Every file `save_lab_files` writes for a lab, built from what the server sent and not yet
/// written.
pub(crate) struct LabFileSet {
    ssh_config: LabFile,
    known_hosts: Option<LabFile>,
    lab_info: LabFile,
    lab_ca: Option<LabFile>,
    vaults: Vec<VaultFile>,
    keycloaks: Vec<KeycloakFile>,
    surreal_dbs: Vec<SurrealDbFile>,
    clusters: Vec<ClusterFiles>,
}

impl LabFileSet {
    /// The files of `files` at their paths in `folder`. Writes nothing.
    ///
    /// # Errors
    ///
    /// Fails when the SSH config cannot name the folder's known hosts file, or a credentials
    /// file cannot be serialized.
    pub(crate) fn new(folder: &LabFolder, files: &LabFiles) -> Result<Self> {
        Ok(Self {
            ssh_config: LabFile {
                path: folder.ssh_config(),
                content: rewrite_ssh_config_for_client(files.ssh_config, folder)?,
                secret: None,
            },
            known_hosts: known_hosts_file(folder, files.known_hosts),
            lab_info: LabFile {
                path: folder.lab_info(),
                content: files.lab_info.to_string(),
                secret: None,
            },
            lab_ca: lab_ca_file(folder, files.lab_ca),
            vaults: vault_credential_files(folder, files.vaults)
                .context("Failed to build Vault credentials")?,
            keycloaks: keycloak_credential_files(folder, files.keycloaks)
                .context("Failed to build Keycloak credentials")?,
            surreal_dbs: surreal_db_credential_files(folder, files.surreal_dbs)
                .context("Failed to build SurrealDB credentials")?,
            clusters: cluster_credential_files(folder, files.clusters),
        })
    }

    /// Every file of the set, in the order they are written.
    pub(crate) fn files(&self) -> impl Iterator<Item = &LabFile> {
        [&self.ssh_config]
            .into_iter()
            .chain(&self.known_hosts)
            .chain([&self.lab_info])
            .chain(&self.lab_ca)
            .chain(self.vaults.iter().map(|vault| &vault.file))
            .chain(self.keycloaks.iter().map(|keycloak| &keycloak.file))
            .chain(self.surreal_dbs.iter().map(|surreal_db| &surreal_db.file))
            .chain(
                self.clusters
                    .iter()
                    .flat_map(|cluster| [&cluster.kubeconfig, &cluster.talosconfig]),
            )
    }

    /// The first file of the set that is already in the lab folder, a symlink or a folder
    /// where it belongs included.
    pub(crate) fn first_existing(&self) -> Option<&Path> {
        self.files()
            .map(|file| file.path.as_path())
            .find(|path| fs::symlink_metadata(path).is_ok())
    }
}

/// Save `files` in `folder`'s `.sherpa` folder, then show each one saved, and return the path
/// of the SSH config. Stops at the first file that cannot be saved.
#[tracing::instrument(level = "debug", skip_all, fields(folder = %folder.root().display()))]
pub(crate) fn save_lab_files(folder: &LabFolder, files: &LabFileSet) -> Result<PathBuf> {
    for file in files.files() {
        write_lab_file(folder, file)?;
    }
    println!(
        "{} SSH config created: {}",
        Emoji::Success,
        files.ssh_config.path.display()
    );
    if let Some(known_hosts) = &files.known_hosts {
        println!(
            "{} Node host keys saved: {}",
            Emoji::Success,
            known_hosts.path.display()
        );
    }
    println!(
        "{} Lab info created: {}",
        Emoji::Success,
        files.lab_info.path.display()
    );
    if let Some(lab_ca) = &files.lab_ca {
        println!(
            "{} Lab CA certificate: {}",
            Emoji::Success,
            lab_ca.path.display()
        );
    }
    print_vault_files(
        &files.vaults,
        files.lab_ca.as_ref().map(|lab_ca| lab_ca.path.as_path()),
    );
    print_keycloak_files(&files.keycloaks);
    print_surreal_db_files(&files.surreal_dbs);
    print_cluster_files(&files.clusters);
    Ok(files.ssh_config.path.clone())
}

/// Write `file` in `folder`, making the folder it goes in, which only its owner can enter. A
/// secret is readable by the user alone.
#[tracing::instrument(level = "debug", skip_all, fields(path = %file.path.display()))]
pub(crate) fn write_lab_file(folder: &LabFolder, file: &LabFile) -> Result<()> {
    let dir = file
        .path
        .parent()
        .with_context(|| format!("{} has no folder", file.path.display()))?;
    ensure_lab_subdir(folder, dir)?;
    match file.secret {
        Some(what) => write_secret_file(&file.path, &file.content, what),
        None => fs::write(&file.path, &file.content)
            .with_context(|| format!("Failed to write {}", file.path.display())),
    }
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
///   path of `folder`'s `.sherpa/ssh/known_hosts`, in double quotes when it holds whitespace,
///   so the config works from anywhere, as through the `~/.ssh/config` include or
///   `ssh -F .sherpa/ssh/config`. The config names no key file: nodes take the user's own keys.
/// - On Windows, replaces `UserKnownHostsFile /dev/null` with `UserKnownHostsFile NUL`
///
/// # Errors
///
/// Fails when the lab folder's path has a double quote, which ssh cannot read.
pub(crate) fn rewrite_ssh_config_for_client(
    ssh_config: &str,
    folder: &LabFolder,
) -> Result<String> {
    let known_hosts = ssh_config_path(&folder.known_hosts().to_string_lossy())?;
    let result = ssh_config.replace(
        &format!("UserKnownHostsFile {SHERPA_KNOWN_HOSTS_FILE}"),
        &format!("UserKnownHostsFile {known_hosts}"),
    );

    #[cfg(windows)]
    let result = result.replace("UserKnownHostsFile /dev/null", "UserKnownHostsFile NUL");

    Ok(result)
}

/// The lab's `known_hosts` file in `folder`, a secret. `None` when the server recorded no host
/// keys.
fn known_hosts_file(folder: &LabFolder, known_hosts: &str) -> Option<LabFile> {
    if known_hosts.is_empty() {
        tracing::debug!("The server recorded no node host keys");
        return None;
    }
    Some(LabFile {
        path: folder.known_hosts(),
        content: known_hosts.to_owned(),
        secret: Some("node host keys"),
    })
}

/// Save the lab's `known_hosts` file in `folder`, and return its path. Nothing is written when
/// the server recorded no host keys. The file is replaced in one step, as nodes being
/// provisioned at the same time read it while another node refreshes it.
#[tracing::instrument(level = "debug", skip_all, fields(folder = %folder.root().display()))]
pub(crate) fn write_known_hosts(folder: &LabFolder, known_hosts: &str) -> Result<Option<PathBuf>> {
    let Some(file) = known_hosts_file(folder, known_hosts) else {
        return Ok(None);
    };
    write_lab_file(folder, &file)?;
    Ok(Some(file.path))
}

/// The lab CA certificate in `folder`. `None` when the server sent none (a lab created before
/// the lab CA, or an older server).
fn lab_ca_file(folder: &LabFolder, lab_ca: &str) -> Option<LabFile> {
    if lab_ca.is_empty() {
        tracing::debug!("The server sent no lab CA certificate");
        return None;
    }
    Some(LabFile {
        path: folder.lab_ca(),
        content: lab_ca.to_owned(),
        secret: None,
    })
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};

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

        let ssh_config = save_lab_files(
            &folder,
            &LabFileSet::new(&folder, &lab_files(&lab_info)).unwrap(),
        )
        .unwrap();

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
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();
        let without_ca = LabFiles {
            lab_ca: "",
            ..lab_files(&lab_info)
        };
        save_lab_files(&folder, &LabFileSet::new(&folder, &without_ca).unwrap()).unwrap();
        assert!(!folder.ca_dir().exists());
        let pem = "-----BEGIN CERTIFICATE-----\nlab\n-----END CERTIFICATE-----\n";
        let with_ca = LabFiles {
            lab_ca: pem,
            ..lab_files(&lab_info)
        };
        save_lab_files(&folder, &LabFileSet::new(&folder, &with_ca).unwrap()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(".sherpa/ca/lab-ca.crt")).unwrap(),
            pem
        );
    }

    #[cfg(unix)]
    #[test]
    fn lab_file_privacy_002_saving_leaves_the_folders_700_and_known_hosts_600() {
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();

        save_lab_files(
            &folder,
            &LabFileSet::new(&folder, &lab_files(&lab_info)).unwrap(),
        )
        .unwrap();

        for path in [folder.state_dir(), folder.ssh_dir(), folder.ca_dir()] {
            assert_eq!(mode(&path), 0o700, "{}", path.display());
        }
        assert_eq!(mode(&folder.known_hosts()), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn lab_file_privacy_004_a_symlinked_sherpa_or_folder_of_it_is_refused() {
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();
        let open_folder = || {
            let elsewhere = tempfile::tempdir().unwrap();
            fs::set_permissions(elsewhere.path(), fs::Permissions::from_mode(0o755)).unwrap();
            elsewhere
        };

        let dir = tempfile::tempdir().unwrap();
        let elsewhere = open_folder();
        symlink(elsewhere.path(), dir.path().join(".sherpa")).unwrap();
        let folder = LabFolder::new(dir.path());
        let files = LabFileSet::new(&folder, &lab_files(&lab_info)).unwrap();
        let error = save_lab_files(&folder, &files).unwrap_err();
        assert!(format!("{error:#}").contains("symlink"), "{error:#}");
        assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
        assert_eq!(mode(elsewhere.path()), 0o755);

        let dir = tempfile::tempdir().unwrap();
        let elsewhere = open_folder();
        fs::create_dir(dir.path().join(".sherpa")).unwrap();
        symlink(elsewhere.path(), dir.path().join(".sherpa/ssh")).unwrap();
        let folder = LabFolder::new(dir.path());
        let files = LabFileSet::new(&folder, &lab_files(&lab_info)).unwrap();
        let error = save_lab_files(&folder, &files).unwrap_err();
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

        let result = rewrite_ssh_config_for_client(server_config, &folder).unwrap();

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
    fn ssh_config_hosts_009_a_lab_folder_with_a_space_names_its_known_hosts_file_in_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(&dir.path().join("my labs"));

        let result = rewrite_ssh_config_for_client(
            "Host dev\n    UserKnownHostsFile sherpa_known_hosts\n",
            &folder,
        )
        .unwrap();

        let expected = format!(
            r#"    UserKnownHostsFile "{}""#,
            folder.known_hosts().display()
        );
        assert_eq!(result.lines().nth(1), Some(expected.as_str()));
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
