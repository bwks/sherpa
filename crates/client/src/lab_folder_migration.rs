//! Move the files older clients left loose in a lab folder into its `.sherpa` folder.
//!
//! Clients before `.sherpa` saved `sherpa_ssh_config`, `sherpa_ssh_key`, `sherpa_known_hosts`,
//! `lab-info.toml`, `lab-ca.crt` and the Vault, Keycloak and cluster credentials beside the
//! manifest. Every command that uses a lab folder runs `migrate_lab_folder` first, so the rest
//! of the client knows only the `.sherpa` layout (`shared::util::LabFolder`). The step is
//! idempotent: a migrated folder, or one that is not a lab's, is left as it is.
//!
//! This module is temporary: remove it, and its calls, in a later release, once labs set up
//! before `.sherpa` are gone.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use topology::Manifest;

use shared::konst::{
    LAB_FILE_NAME, SHERPA_KNOWN_HOSTS_FILE, SHERPA_LAB_CA_FILE, SHERPA_MANIFEST_FILE,
    SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_PRIVATE_KEY_FILE,
};
use shared::util::{
    Emoji, LabFolder, get_cwd, get_user_ssh_dir, keycloak_file_name, kubeconfig_file_name,
    replace_lab_ssh_include, talosconfig_file_name, vault_file_name,
};

use crate::lab_files::ensure_lab_subdir;
use crate::private_key::restrict_secret_permissions;

/// What a failed migration of a lab folder is reported as.
pub(crate) const MIGRATION_FAILED: &str = "Failed to move the lab files into .sherpa";
/// The keys of the `vault-<node>.toml` older clients wrote, to tell it from a user's file.
const VAULT_FILE_KEYS: &[&str] = &["address", "root_token", "unseal_key"];
/// The keys of the `keycloak-<node>.toml` older clients wrote.
const KEYCLOAK_FILE_KEYS: &[&str] = &["account_console", "address", "issuer", "realm", "users"];

/// What a migration did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Migration {
    /// The new path of each file moved into `.sherpa`.
    pub moved: Vec<PathBuf>,
    /// Whether `~/.ssh/sherpa_lab_hosts` now includes `.sherpa/ssh/config` instead of the old
    /// `sherpa_ssh_config`.
    pub include_replaced: bool,
}

/// One loose file and where it goes.
struct Move {
    from: PathBuf,
    to: PathBuf,
    secret: bool,
}

/// Migrate the current folder, with the user's `~/.ssh`, and show what moved.
///
/// # Errors
///
/// Fails when the current folder or home folder is unknown, or `migrate_lab_folder` fails.
#[tracing::instrument(level = "debug")]
pub(crate) fn migrate_current_lab_folder() -> Result<()> {
    let cwd = get_cwd().context("Failed to determine working directory")?;
    let ssh_dir = get_user_ssh_dir()?;
    let migration = migrate_lab_folder(Path::new(&cwd), &ssh_dir)?;
    print_migration(&migration);
    Ok(())
}

/// Migrate each lab folder an old `sherpa_ssh_config` in `include_paths` (the Include targets of
/// `~/.ssh/sherpa_lab_hosts`) belongs to. A folder that cannot be migrated only warns.
#[tracing::instrument(level = "debug", skip(include_paths), fields(ssh_dir = %ssh_dir.display()))]
pub(crate) fn migrate_registered_lab_folders(ssh_dir: &Path, include_paths: &[String]) {
    for lab_dir in legacy_lab_folders(include_paths) {
        match migrate_lab_folder(&lab_dir, ssh_dir) {
            Ok(migration) => print_migration(&migration),
            Err(error) => eprintln!(
                "{} Failed to move the lab files in {} into .sherpa: {error:#}",
                Emoji::Warning,
                lab_dir.display()
            ),
        }
    }
}

/// Move the files older clients generated in `lab_dir` into `lab_dir/.sherpa`, owner only, and
/// point the lab's `Include` in `ssh_dir/sherpa_lab_hosts` at `.sherpa/ssh/config`.
///
/// Only a lab folder is touched: one with a loose `lab-info.toml` or `sherpa_ssh_config`, or a
/// `.sherpa` folder from an earlier, partial migration. Only files the client generated move:
/// Vault and Keycloak files whose contents are the client's, and the credentials of clusters
/// in the manifest or with both a kubeconfig and a talosconfig. A file already in `.sherpa` is
/// replaced by a loose one, which an older client wrote later.
///
/// # Errors
///
/// Fails when `.sherpa` is a symlink or not a folder, or a file cannot be moved.
#[tracing::instrument(level = "debug", fields(lab_dir = %lab_dir.display(), ssh_dir = %ssh_dir.display()))]
pub(crate) fn migrate_lab_folder(lab_dir: &Path, ssh_dir: &Path) -> Result<Migration> {
    let folder = LabFolder::new(lab_dir);
    let state_dir = folder.state_dir();
    let state_exists = match fs::symlink_metadata(&state_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() => bail!(
            "{} is a symlink; Sherpa keeps lab files only in a real folder",
            state_dir.display()
        ),
        Ok(metadata) if !metadata.is_dir() => bail!("{} is not a folder", state_dir.display()),
        Ok(_) => true,
        Err(error) if error.kind() == ErrorKind::NotFound => false,
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", state_dir.display()));
        }
    };
    let legacy_config = lab_dir.join(SHERPA_SSH_CONFIG_FILE);
    let is_lab = state_exists || is_file(&legacy_config) || is_file(&lab_dir.join(LAB_FILE_NAME));
    if !is_lab {
        return Ok(Migration::default());
    }

    let mut migration = Migration::default();
    for planned in planned_moves(lab_dir, &folder) {
        if !is_file(&planned.from) {
            continue;
        }
        let parent = planned
            .to
            .parent()
            .with_context(|| format!("{} has no folder", planned.to.display()))?;
        ensure_lab_subdir(&folder, parent)?;
        fs::rename(&planned.from, &planned.to).with_context(|| {
            format!(
                "Failed to move {} to {}",
                planned.from.display(),
                planned.to.display()
            )
        })?;
        if planned.secret {
            restrict_secret_permissions(&planned.to, "lab file")?;
        }
        migration.moved.push(planned.to);
    }

    if is_file(&legacy_config) {
        let config = fs::read_to_string(&legacy_config)
            .with_context(|| format!("Failed to read {}", legacy_config.display()))?;
        ensure_lab_subdir(&folder, &folder.ssh_dir())?;
        let new_config = folder.ssh_config();
        fs::write(&new_config, rewrite_legacy_ssh_config(&config, &folder))
            .with_context(|| format!("Failed to write {}", new_config.display()))?;
        fs::remove_file(&legacy_config)
            .with_context(|| format!("Failed to remove {}", legacy_config.display()))?;
        migration.moved.push(new_config);
    }

    // The lab-info file goes last, so a migration cut short is still seen as a lab's.
    let legacy_lab_info = lab_dir.join(LAB_FILE_NAME);
    if is_file(&legacy_lab_info) {
        ensure_lab_subdir(&folder, &state_dir)?;
        fs::rename(&legacy_lab_info, folder.lab_info()).with_context(|| {
            format!("Failed to move {} into .sherpa", legacy_lab_info.display())
        })?;
        migration.moved.push(folder.lab_info());
    }

    migration.include_replaced = replace_lab_ssh_include(
        ssh_dir,
        &legacy_config.to_string_lossy(),
        &folder.ssh_config().to_string_lossy(),
    )?;
    Ok(migration)
}

/// The loose files older clients generated in `lab_dir`, by their exact names: the ones
/// `migrate_lab_folder` would move. Nothing else is listed, and a symlink is not a file.
pub(crate) fn legacy_lab_files(lab_dir: &Path) -> Vec<PathBuf> {
    let folder = LabFolder::new(lab_dir);
    let mut files = vec![
        lab_dir.join(SHERPA_SSH_CONFIG_FILE),
        lab_dir.join(LAB_FILE_NAME),
    ];
    files.extend(planned_moves(lab_dir, &folder).into_iter().map(|m| m.from));
    files.retain(|path| is_file(path));
    files
}

/// The lab folders of the old `sherpa_ssh_config` files among `include_paths`.
fn legacy_lab_folders(include_paths: &[String]) -> Vec<PathBuf> {
    include_paths
        .iter()
        .map(Path::new)
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name == SHERPA_SSH_CONFIG_FILE)
        })
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect()
}

/// The loose files older clients generated in `lab_dir`, other than the SSH config and
/// `lab-info.toml`, and where each goes in `folder`.
fn planned_moves(lab_dir: &Path, folder: &LabFolder) -> Vec<Move> {
    let mut moves = vec![
        Move {
            from: lab_dir.join(SHERPA_SSH_PRIVATE_KEY_FILE),
            to: folder.ssh_key(),
            secret: true,
        },
        Move {
            from: lab_dir.join(SHERPA_KNOWN_HOSTS_FILE),
            to: folder.known_hosts(),
            secret: true,
        },
        Move {
            from: lab_dir.join(SHERPA_LAB_CA_FILE),
            to: folder.lab_ca(),
            secret: false,
        },
    ];
    for node in generated_names(lab_dir, "vault-", ".toml", |content| {
        has_exactly_keys(content, VAULT_FILE_KEYS)
    }) {
        moves.push(Move {
            from: lab_dir.join(vault_file_name(&node)),
            to: folder.vault(&node),
            secret: true,
        });
    }
    for node in generated_names(lab_dir, "keycloak-", ".toml", |content| {
        has_exactly_keys(content, KEYCLOAK_FILE_KEYS)
    }) {
        moves.push(Move {
            from: lab_dir.join(keycloak_file_name(&node)),
            to: folder.keycloak(&node),
            secret: true,
        });
    }
    for cluster in cluster_names(lab_dir) {
        moves.push(Move {
            from: lab_dir.join(kubeconfig_file_name(&cluster)),
            to: folder.kubeconfig(&cluster),
            secret: true,
        });
        moves.push(Move {
            from: lab_dir.join(talosconfig_file_name(&cluster)),
            to: folder.talosconfig(&cluster),
            secret: true,
        });
    }
    moves
}

/// The `<name>` of each file `<prefix><name><suffix>` in `dir` whose contents pass `generated`.
fn generated_names(
    dir: &Path,
    prefix: &str,
    suffix: &str,
    generated: impl Fn(&str) -> bool,
) -> Vec<String> {
    file_names(dir)
        .into_iter()
        .filter_map(|file| {
            let name = file.strip_prefix(prefix)?.strip_suffix(suffix)?.to_string();
            let content = fs::read_to_string(dir.join(&file)).ok()?;
            generated(&content).then_some(name)
        })
        .collect()
}

/// Whether `content` is a TOML table with exactly the sorted top-level `keys`.
fn has_exactly_keys(content: &str, keys: &[&str]) -> bool {
    toml::from_str::<toml::Table>(content).is_ok_and(|table| {
        let mut found: Vec<&str> = table.keys().map(String::as_str).collect();
        found.sort_unstable();
        found == keys
    })
}

/// The Talos clusters whose credentials an older client wrote in `dir`: those in its manifest,
/// and any with both a `kubeconfig-<cluster>` and a `talosconfig-<cluster>`, which the client
/// always wrote together.
fn cluster_names(dir: &Path) -> Vec<String> {
    let names = file_names(dir);
    let mut clusters: Vec<String> = names
        .iter()
        .filter_map(|file| file.strip_prefix("kubeconfig-"))
        .filter(|cluster| names.contains(&talosconfig_file_name(cluster)))
        .map(ToString::to_string)
        .collect();
    let manifest = dir.join(SHERPA_MANIFEST_FILE);
    if is_file(&manifest) {
        match Manifest::load_file(&manifest.to_string_lossy()) {
            Ok(manifest) => clusters.extend(
                manifest
                    .talos_clusters
                    .unwrap_or_default()
                    .into_iter()
                    .map(|cluster| cluster.name),
            ),
            Err(error) => {
                tracing::debug!(error = %format!("{error:#}"), "Ignoring a manifest that does not load");
            }
        }
    }
    clusters.sort();
    clusters.dedup();
    clusters
}

/// The names of the files directly in `dir`.
fn file_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

/// Whether `path` is a regular file, not following a symlink.
fn is_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// Point an old SSH config's `IdentityFile` and `UserKnownHostsFile` lines, which named the
/// loose `sherpa_ssh_key` and `sherpa_known_hosts` (by absolute path, or relative in the oldest
/// clients), at `folder`'s `.sherpa/ssh/key` and `.sherpa/ssh/known_hosts`.
fn rewrite_legacy_ssh_config(config: &str, folder: &LabFolder) -> String {
    let mut rewritten: String = config
        .lines()
        .map(|line| {
            let indent = &line[..line.len() - line.trim_start().len()];
            let mut fields = line.trim_start().splitn(2, char::is_whitespace);
            let directive = fields.next().unwrap_or_default();
            let value = fields.next().unwrap_or_default().trim().trim_matches('"');
            let names = |file: &str| {
                Path::new(value)
                    .file_name()
                    .is_some_and(|name| name == file)
            };
            if directive.eq_ignore_ascii_case("IdentityFile") && names(SHERPA_SSH_PRIVATE_KEY_FILE)
            {
                format!("{indent}IdentityFile {}", folder.ssh_key().display())
            } else if directive.eq_ignore_ascii_case("UserKnownHostsFile")
                && names(SHERPA_KNOWN_HOSTS_FILE)
            {
                format!(
                    "{indent}UserKnownHostsFile {}",
                    folder.known_hosts().display()
                )
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if config.ends_with('\n') {
        rewritten.push('\n');
    }
    rewritten
}

/// Show what a migration moved.
fn print_migration(migration: &Migration) {
    if !migration.moved.is_empty() {
        println!(
            "{} Moved {} lab files into .sherpa",
            Emoji::Success,
            migration.moved.len()
        );
    }
    if migration.include_replaced {
        println!(
            "{} ~/.ssh/sherpa_lab_hosts now includes .sherpa/ssh/config",
            Emoji::Success
        );
    }
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
    const VAULT: &str = "address = \"https://v\"\nroot_token = \"t\"\nunseal_key = \"k\"\n";
    const KEYCLOAK: &str = "address = \"https://k\"\nrealm = \"r\"\nissuer = \"i\"\n\
                            account_console = \"a\"\nusers = []\n";

    /// A lab folder as an older client left it, registered in `ssh_dir`, with files of the
    /// user's beside it.
    fn old_layout(lab_dir: &Path, ssh_dir: &Path) {
        let key = lab_dir.join("sherpa_ssh_key");
        let known_hosts = lab_dir.join("sherpa_known_hosts");
        let config = format!(
            "Host dev.fecb1a12\n    HostName 172.31.3.21\n    IdentityFile {}\n    \
             StrictHostKeyChecking yes\n    UserKnownHostsFile {}\n",
            key.display(),
            known_hosts.display()
        );
        let files = [
            ("sherpa_ssh_config", config.as_str()),
            ("sherpa_ssh_key", "private key"),
            ("sherpa_known_hosts", "172.31.3.21 ssh-ed25519 AAAA\n"),
            ("lab-info.toml", LAB_INFO),
            ("lab-ca.crt", "-----BEGIN CERTIFICATE-----\n"),
            ("vault-vault01.toml", VAULT),
            ("keycloak-kc01.toml", KEYCLOAK),
            ("kubeconfig-k8s", "kube"),
            ("talosconfig-k8s", "talos"),
            // The user's own files.
            ("manifest.toml", "name = \"sherpa-dev\"\n"),
            ("notes.txt", "mine"),
            ("vault-notes.toml", "title = \"mine\"\n"),
            ("kubeconfig-prod", "mine"),
        ];
        for (name, content) in files {
            fs::write(lab_dir.join(name), content).unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
        }
        fs::create_dir_all(ssh_dir).unwrap();
        fs::write(
            ssh_dir.join("sherpa_lab_hosts"),
            format!(
                "# header\nInclude {}\nInclude /other/lab/.sherpa/ssh/config\n",
                lab_dir.join("sherpa_ssh_config").display()
            ),
        )
        .unwrap();
    }

    fn sorted_names(dir: &Path) -> Vec<String> {
        let mut names = file_names(dir);
        names.sort();
        names
    }

    #[test]
    fn an_old_layout_moves_into_the_sherpa_folder() {
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let ssh_dir = home.path().join(".ssh");
        old_layout(lab.path(), &ssh_dir);
        let folder = LabFolder::new(lab.path());

        let migration = migrate_lab_folder(lab.path(), &ssh_dir).unwrap();

        assert_eq!(migration.moved.len(), 9, "{:?}", migration.moved);
        assert!(migration.include_replaced);
        assert_eq!(fs::read_to_string(folder.ssh_key()).unwrap(), "private key");
        assert_eq!(fs::read_to_string(folder.lab_info()).unwrap(), LAB_INFO);
        for path in [
            folder.known_hosts(),
            folder.lab_ca(),
            folder.vault("vault01"),
            folder.keycloak("kc01"),
            folder.kubeconfig("k8s"),
            folder.talosconfig("k8s"),
        ] {
            assert!(path.is_file(), "{}", path.display());
        }
        assert_eq!(
            sorted_names(lab.path()),
            [
                "kubeconfig-prod",
                "manifest.toml",
                "notes.txt",
                "vault-notes.toml"
            ],
            "only the user's files stay loose"
        );
        let target = jato::SshConfig::load_file(folder.ssh_config())
            .unwrap()
            .route("dev.fecb1a12")
            .unwrap()
            .target;
        assert_eq!(target.identity_files, [folder.ssh_key()]);
        assert_eq!(target.known_hosts, [folder.known_hosts()]);
        let index = fs::read_to_string(ssh_dir.join("sherpa_lab_hosts")).unwrap();
        assert_eq!(
            index,
            format!(
                "# header\nInclude {}\nInclude /other/lab/.sherpa/ssh/config\n",
                folder.ssh_config().display()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn migrated_folders_are_owner_only_and_secrets_0600() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        old_layout(lab.path(), &home.path().join(".ssh"));
        let folder = LabFolder::new(lab.path());

        migrate_lab_folder(lab.path(), &home.path().join(".ssh")).unwrap();

        for dir in [
            folder.state_dir(),
            folder.ssh_dir(),
            folder.ca_dir(),
            folder.vault_dir(),
            folder.keycloak_dir(),
            folder.k8s_dir(),
            folder.talos_dir(),
        ] {
            assert_eq!(mode(&dir), 0o700, "{}", dir.display());
        }
        for secret in [
            folder.ssh_key(),
            folder.known_hosts(),
            folder.vault("vault01"),
            folder.keycloak("kc01"),
            folder.kubeconfig("k8s"),
            folder.talosconfig("k8s"),
        ] {
            assert_eq!(mode(&secret), 0o600, "{}", secret.display());
        }
    }

    #[test]
    fn legacy_lab_files_lists_only_what_the_old_client_generated() {
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        old_layout(lab.path(), &home.path().join(".ssh"));

        let mut names: Vec<String> = legacy_lab_files(lab.path())
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();

        assert_eq!(
            names,
            [
                "keycloak-kc01.toml",
                "kubeconfig-k8s",
                "lab-ca.crt",
                "lab-info.toml",
                "sherpa_known_hosts",
                "sherpa_ssh_config",
                "sherpa_ssh_key",
                "talosconfig-k8s",
                "vault-vault01.toml",
            ]
        );
    }

    #[test]
    fn a_migrated_folder_is_left_as_it_is() {
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let ssh_dir = home.path().join(".ssh");
        old_layout(lab.path(), &ssh_dir);
        migrate_lab_folder(lab.path(), &ssh_dir).unwrap();
        let folder = LabFolder::new(lab.path());
        let config = fs::read_to_string(folder.ssh_config()).unwrap();
        let index = fs::read_to_string(ssh_dir.join("sherpa_lab_hosts")).unwrap();

        let again = migrate_lab_folder(lab.path(), &ssh_dir).unwrap();

        assert_eq!(again, Migration::default());
        assert_eq!(fs::read_to_string(folder.ssh_config()).unwrap(), config);
        assert_eq!(
            fs::read_to_string(ssh_dir.join("sherpa_lab_hosts")).unwrap(),
            index
        );
    }

    #[test]
    fn a_partial_migration_is_finished() {
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let ssh_dir = home.path().join(".ssh");
        old_layout(lab.path(), &ssh_dir);
        let folder = LabFolder::new(lab.path());
        // Cut short after lab-info.toml and the key moved; the loose key is newer.
        fs::create_dir_all(folder.ssh_dir()).unwrap();
        fs::rename(lab.path().join("lab-info.toml"), folder.lab_info()).unwrap();
        fs::write(folder.ssh_key(), "older key").unwrap();

        let migration = migrate_lab_folder(lab.path(), &ssh_dir).unwrap();

        assert_eq!(migration.moved.len(), 8, "{:?}", migration.moved);
        assert!(migration.include_replaced);
        assert_eq!(fs::read_to_string(folder.lab_info()).unwrap(), LAB_INFO);
        assert_eq!(fs::read_to_string(folder.ssh_key()).unwrap(), "private key");
        assert!(folder.ssh_config().is_file());
        assert!(!lab.path().join("sherpa_ssh_config").exists());
    }

    #[test]
    fn a_swapped_include_is_finished_once_the_files_have_moved() {
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let ssh_dir = home.path().join(".ssh");
        old_layout(lab.path(), &ssh_dir);
        let folder = LabFolder::new(lab.path());
        fs::create_dir_all(folder.ssh_dir()).unwrap();
        fs::rename(lab.path().join("sherpa_ssh_config"), folder.ssh_config()).unwrap();
        fs::rename(lab.path().join("lab-info.toml"), folder.lab_info()).unwrap();

        let migration = migrate_lab_folder(lab.path(), &ssh_dir).unwrap();

        assert!(migration.include_replaced);
        let index = fs::read_to_string(ssh_dir.join("sherpa_lab_hosts")).unwrap();
        assert!(index.contains(&folder.ssh_config().display().to_string()));
        assert!(!index.contains("sherpa_ssh_config"));
    }

    #[test]
    fn a_folder_that_is_not_a_lab_is_untouched() {
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        for name in ["kubeconfig-a", "talosconfig-a", "sherpa_ssh_key"] {
            fs::write(lab.path().join(name), "x").unwrap();
        }

        let migration = migrate_lab_folder(lab.path(), &home.path().join(".ssh")).unwrap();

        assert_eq!(migration, Migration::default());
        assert!(!lab.path().join(".sherpa").exists());
        assert_eq!(sorted_names(lab.path()).len(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_sherpa_folder_is_refused() {
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        old_layout(lab.path(), &home.path().join(".ssh"));
        std::os::unix::fs::symlink(elsewhere.path(), lab.path().join(".sherpa")).unwrap();

        let error = migrate_lab_folder(lab.path(), &home.path().join(".ssh")).unwrap_err();

        assert!(error.to_string().contains("symlink"), "{error}");
        assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
        assert!(lab.path().join("sherpa_ssh_key").is_file());
        assert!(lab.path().join("lab-info.toml").is_file());
    }

    #[test]
    fn only_old_ssh_configs_name_lab_folders_to_migrate() {
        let includes = [
            "/labs/a/sherpa_ssh_config".to_string(),
            "/labs/b/.sherpa/ssh/config".to_string(),
            "/labs/c/other".to_string(),
        ];
        assert_eq!(legacy_lab_folders(&includes), [PathBuf::from("/labs/a")]);
    }

    #[test]
    fn an_old_relative_ssh_config_is_pointed_at_the_sherpa_folder() {
        let folder = LabFolder::new(Path::new("/labs/a"));
        let config = "Host x\n  IdentityFile sherpa_ssh_key\n  UserKnownHostsFile /dev/null\n";

        let rewritten = rewrite_legacy_ssh_config(config, &folder);

        assert_eq!(
            rewritten,
            format!(
                "Host x\n  IdentityFile {}\n  UserKnownHostsFile /dev/null\n",
                folder.ssh_key().display()
            )
        );
    }
}
