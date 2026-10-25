//! Container secret files, kept in a RAM-backed server directory and bind-mounted read-only at
//! `/run/secrets` in the container.
//!
//! `<root>/<lab>` is 0700, so only the daemon's user (and root) can reach a lab's files on the
//! server. The node directory is the mount root, so its mode is what the container sees.
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::instrument;

use container::get_container_mount_source;
use shared::data::{NodeKind, NodeSecretsRequest, NodeSecretsResponse, SecretFile};
use shared::konst::{CONTAINER_SECRETS_DIR, SHERPA_SECRETS_DIR};

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_owner;
use crate::daemon::state::AppState;
use crate::daemon::storage::restrict_dir;
use crate::services::error::ServiceError;
use crate::validate::validate_secret_files;

/// Prefix of a file being written; `~` cannot appear in a secret file name.
const TEMP_PREFIX: &str = ".sherpa~";

/// A secret file decoded and checked, ready to write.
struct DecodedSecret<'a> {
    name: &'a str,
    mode: u32,
    content: Vec<u8>,
}

/// Write a node's secret files under `root`, replacing any it had, and return the node's
/// directory, which is mounted in the container.
#[instrument(skip(files), fields(%lab_id, %node_name, files = files.len()), level = "debug")]
pub fn write_node_secrets(
    root: &Path,
    lab_id: &str,
    node_name: &str,
    files: &[SecretFile],
) -> Result<PathBuf> {
    // Check every file before changing anything.
    validate_secret_files(files)?;
    let mut names = HashSet::new();
    let mut decoded = Vec::with_capacity(files.len());
    for file in files {
        names.insert(file.name.as_str());
        let content = file
            .content
            .decode()
            .with_context(|| format!("Secret file '{}' is not valid base64", file.name))?;
        let mode = u32::from_str_radix(&file.mode.to_string(), 8)
            .with_context(|| format!("Secret file '{}' has an invalid mode", file.name))?;
        decoded.push(DecodedSecret {
            name: &file.name,
            mode,
            content,
        });
    }

    restrict_dir(root, 0o700, None).with_context(|| {
        format!(
            "Cannot create {}; sherpad needs its systemd RuntimeDirectory (/run/sherpa), which \
             the update script adds",
            root.display()
        )
    })?;
    let lab_dir = root.join(lab_id);
    restrict_dir(&lab_dir, 0o700, None)?;
    let dir = node_secrets_dir(root, lab_id, node_name);
    restrict_dir(&dir, 0o755, None)?;

    for secret in &decoded {
        write_secret(&dir, secret)?;
    }
    remove_stale(&dir, &names)?;
    Ok(dir)
}

/// Write a file under a temporary name, private until complete, then rename it into place, so
/// the container never sees a partial file.
fn write_secret(dir: &Path, secret: &DecodedSecret) -> Result<()> {
    let temp = dir.join(format!("{TEMP_PREFIX}{}", secret.name));
    match fs::remove_file(&temp) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot remove {}", temp.display()));
        }
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .with_context(|| format!("Cannot create secret file '{}'", secret.name))?;
    file.write_all(&secret.content)
        .with_context(|| format!("Cannot write secret file '{}'", secret.name))?;
    fs::set_permissions(&temp, fs::Permissions::from_mode(secret.mode))
        .with_context(|| format!("Cannot set the mode of secret file '{}'", secret.name))?;
    fs::rename(&temp, dir.join(secret.name))
        .with_context(|| format!("Cannot move secret file '{}' into place", secret.name))
}

/// Remove files that are no longer in the node's secret files.
fn remove_stale(dir: &Path, names: &HashSet<&str>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("Cannot read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("Cannot read {}", dir.display()))?;
        let name = entry.file_name();
        let kept = match name.to_str() {
            Some(name) => names.contains(name),
            None => false,
        };
        if !kept {
            fs::remove_file(entry.path())
                .with_context(|| format!("Cannot remove {}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// The server directory holding a node's secret files.
pub fn node_secrets_dir(root: &Path, lab_id: &str, node_name: &str) -> PathBuf {
    root.join(lab_id).join(node_name)
}

/// The secret files directory to mount in a redeployed container, if any. `Some(files)`
/// replaces the node's files, and an empty list removes them; `None`, from a redeploy that
/// cannot send secrets, keeps the files the node has.
#[instrument(skip(secrets), fields(%lab_id, %node_name), level = "debug")]
pub fn secrets_for_redeploy(
    root: &Path,
    lab_id: &str,
    node_name: &str,
    secrets: Option<&[SecretFile]>,
) -> Result<Option<PathBuf>> {
    let dir = node_secrets_dir(root, lab_id, node_name);
    match secrets {
        Some([]) => {
            match fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("Cannot remove {}", dir.display()));
                }
            }
            Ok(None)
        }
        Some(files) => write_node_secrets(root, lab_id, node_name, files).map(Some),
        None => Ok(dir.is_dir().then_some(dir)),
    }
}

/// Rewrite a running lab's container secret files, for `sherpa provision`. The container must
/// have been created with them, as its `/run/secrets` mount is fixed when it is created.
#[instrument(skip(ctx, request, state), fields(username = %ctx.username, lab_id = %request.lab_id, node_name = %request.node_name))]
pub async fn update_node_secrets(
    ctx: &AuthContext,
    request: NodeSecretsRequest,
    state: &AppState,
) -> Result<NodeSecretsResponse> {
    let lab_id = &request.lab_id;
    let node_name = &request.node_name;
    let db_lab = require_lab_owner(ctx, &state.db, lab_id).await?;
    let lab_record_id = db::get_lab_id(&db_lab).context("Failed to get lab record ID")?;
    let db_node = match db::get_node_by_name_and_lab(&state.db, node_name, lab_record_id).await {
        Ok(db_node) => db_node,
        Err(error) => {
            // The lookup returns one error for a missing node and a failed query.
            tracing::warn!(%lab_id, %node_name, error = %format!("{error:#}"), "Node lookup failed");
            return Err(ServiceError::NotFound(format!(
                "Node '{node_name}' not found in lab '{lab_id}'"
            ))
            .into());
        }
    };
    let node_images = db::list_node_images_by_ids(&state.db, vec![db_node.image.clone()])
        .await
        .context("Failed to get node image config")?;
    let node_image = node_images
        .first()
        .with_context(|| format!("Node image config not found for node '{node_name}'"))?;
    if node_image.kind != NodeKind::Container {
        return Err(ServiceError::Conflict(format!(
            "Node '{node_name}' is not a container, so it takes no secret files"
        ))
        .into());
    }
    if let Err(error) = validate_secret_files(&request.files) {
        return Err(ServiceError::Conflict(format!("Node '{node_name}': {error:#}")).into());
    }

    let root = Path::new(SHERPA_SECRETS_DIR);
    let mount = get_container_mount_source(
        &state.docker,
        &format!("{node_name}-{lab_id}"),
        CONTAINER_SECRETS_DIR,
    )
    .await?;
    check_secrets_mount(
        mount.as_deref(),
        &node_secrets_dir(root, lab_id, node_name),
        node_name,
    )?;

    write_node_secrets(root, lab_id, node_name, &request.files)?;
    Ok(NodeSecretsResponse {
        node_name: node_name.clone(),
        files_written: request.files.len(),
    })
}

/// Check that a container mounts its secret files from the node's directory.
fn check_secrets_mount(source: Option<&str>, expected: &Path, node_name: &str) -> Result<()> {
    match source {
        Some(source) if Path::new(source) == expected => Ok(()),
        Some(source) => Err(ServiceError::Conflict(format!(
            "Container '{node_name}' mounts {source} at {CONTAINER_SECRETS_DIR}, not its secret \
             files"
        ))
        .into()),
        None => Err(ServiceError::Conflict(format!(
            "Container '{node_name}' was created without secret files; run `sherpa redeploy -n \
             {node_name}` to add them"
        ))
        .into()),
    }
}

/// Remove a lab's secret files.
#[instrument(fields(%lab_id), level = "debug")]
pub fn remove_lab_secrets(root: &Path, lab_id: &str) -> Result<()> {
    let dir = root.join(lab_id);
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Cannot remove {}", dir.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use shared::data::SecretContent;
    use tempfile::tempdir;

    fn secret(name: &str, mode: u32, content: &[u8]) -> SecretFile {
        SecretFile {
            name: name.to_string(),
            mode,
            content: SecretContent::from_bytes(content),
        }
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn files_are_written_with_their_modes_in_a_private_lab_dir() {
        let root = tempdir().unwrap();
        let secrets = root.path().join("secrets");
        let dir = write_node_secrets(
            &secrets,
            "abcd1234",
            "db01",
            &[
                secret("password", 444, b"pw"),
                secret("tls.key", 400, &[0, 255]),
            ],
        )
        .unwrap();
        assert_eq!(dir, secrets.join("abcd1234/db01"));
        assert_eq!(fs::read(dir.join("password")).unwrap(), b"pw");
        assert_eq!(fs::read(dir.join("tls.key")).unwrap(), [0, 255]);
        assert_eq!(mode(&dir.join("password")), 0o444);
        assert_eq!(mode(&dir.join("tls.key")), 0o400);
        assert_eq!(mode(&secrets), 0o700);
        assert_eq!(mode(&secrets.join("abcd1234")), 0o700);
        assert_eq!(mode(&dir), 0o755);
    }

    #[test]
    fn rewriting_replaces_files_and_removes_stale_ones() {
        let root = tempdir().unwrap();
        let dir = write_node_secrets(
            root.path(),
            "abcd1234",
            "db01",
            &[secret("old", 444, b"1"), secret("kept", 444, b"1")],
        )
        .unwrap();
        // A read-only file must still be replaceable.
        write_node_secrets(
            root.path(),
            "abcd1234",
            "db01",
            &[secret("kept", 400, b"2")],
        )
        .unwrap();
        assert!(!dir.join("old").exists());
        assert_eq!(fs::read(dir.join("kept")).unwrap(), b"2");
        assert_eq!(mode(&dir.join("kept")), 0o400);
        let names: Vec<_> = fs::read_dir(&dir).unwrap().collect();
        assert_eq!(names.len(), 1, "no temporary files are left");
    }

    #[test]
    fn invalid_files_change_nothing() {
        let root = tempdir().unwrap();
        let dir =
            write_node_secrets(root.path(), "abcd1234", "db01", &[secret("a", 444, b"1")]).unwrap();
        let bad_content: SecretContent = serde_json::from_value(serde_json::json!("!!")).unwrap();
        for files in [
            vec![secret("../escape", 444, b"x")],
            vec![secret("b", 448, b"x")],
            vec![SecretFile {
                name: "b".to_string(),
                mode: 444,
                content: bad_content.clone(),
            }],
            vec![secret("b", 444, b"x"), secret("b", 400, b"y")],
        ] {
            assert!(write_node_secrets(root.path(), "abcd1234", "db01", &files).is_err());
            assert_eq!(fs::read(dir.join("a")).unwrap(), b"1");
            assert!(!dir.join("b").exists());
        }
        assert!(!root.path().join("escape").exists());
    }

    #[test]
    fn redeploys_replace_remove_or_keep_secrets() {
        let root = tempdir().unwrap();
        let dir = node_secrets_dir(root.path(), "abcd1234", "db01");

        // Nothing sent and nothing kept: no mount.
        assert_eq!(
            secrets_for_redeploy(root.path(), "abcd1234", "db01", None).unwrap(),
            None
        );

        let files = [secret("password", 444, b"1")];
        assert_eq!(
            secrets_for_redeploy(root.path(), "abcd1234", "db01", Some(&files)).unwrap(),
            Some(dir.clone())
        );
        assert_eq!(fs::read(dir.join("password")).unwrap(), b"1");

        // The web UI sends none, so the node keeps its files.
        assert_eq!(
            secrets_for_redeploy(root.path(), "abcd1234", "db01", None).unwrap(),
            Some(dir.clone())
        );
        assert_eq!(fs::read(dir.join("password")).unwrap(), b"1");

        // The CLI sends an empty list when the provisioner was removed.
        assert_eq!(
            secrets_for_redeploy(root.path(), "abcd1234", "db01", Some(&[])).unwrap(),
            None
        );
        assert!(!dir.exists());
    }

    #[test]
    fn secrets_are_rewritten_only_through_the_nodes_own_mount() {
        let expected = Path::new("/run/sherpa/secrets/abcd1234/db01");
        check_secrets_mount(Some("/run/sherpa/secrets/abcd1234/db01"), expected, "db01").unwrap();
        let missing = check_secrets_mount(None, expected, "db01").unwrap_err();
        // A conflict, so the RPC returns the message the user can act on.
        assert!(
            matches!(
                ServiceError::find(&missing),
                Some(ServiceError::Conflict(_))
            ),
            "{missing}"
        );
        let missing = missing.to_string();
        assert!(missing.contains("sherpa redeploy -n db01"), "{missing}");
        let other = check_secrets_mount(Some("/srv/other"), expected, "db01")
            .unwrap_err()
            .to_string();
        assert!(other.contains("/srv/other"), "{other}");
    }

    #[test]
    fn removing_a_lab_removes_its_files_only() {
        let root = tempdir().unwrap();
        write_node_secrets(root.path(), "abcd1234", "db01", &[secret("a", 444, b"1")]).unwrap();
        let other =
            write_node_secrets(root.path(), "ffff0000", "db01", &[secret("a", 444, b"1")]).unwrap();
        remove_lab_secrets(root.path(), "abcd1234").unwrap();
        assert!(!root.path().join("abcd1234").exists());
        assert!(other.join("a").exists());
        // Removing again, or a lab that never had secrets, is not an error.
        remove_lab_secrets(root.path(), "abcd1234").unwrap();
    }
}
