use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

/// Name of an SSH private key in error messages, e.g. "Failed to write SSH private key: <path>".
pub(crate) const SSH_PRIVATE_KEY_ERROR_LABEL: &str = "SSH private key";

/// Write a file holding a secret, such as an SSH private key or a kubeconfig, readable only by
/// the current user. The contents go to a new private file beside `path` that then replaces
/// it, so they are never readable by others, and a symlink at `path` is replaced rather than
/// written through. The new file is removed if anything fails.
///
/// `what` names the file in error messages.
#[tracing::instrument(level = "debug", skip(contents), fields(path = %path.display(), %what))]
pub(crate) fn write_secret_file(path: &Path, contents: &str, what: &str) -> Result<()> {
    let file_name = path
        .file_name()
        .with_context(|| format!("{what} path names no file: {}", path.display()))?;
    let mut temp_name = OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(".partial");
    let temp = path.with_file_name(temp_name);

    let written = write_private_file(&temp, contents)
        .and_then(|()| fs::rename(&temp, path))
        .with_context(|| format!("Failed to write {what}: {}", path.display()));
    if let Err(error) = written {
        return match fs::remove_file(&temp) {
            Ok(()) => Err(error),
            Err(cleanup_error) if cleanup_error.kind() == ErrorKind::NotFound => Err(error),
            Err(cleanup_error) => Err(anyhow!(
                "{error:#}; also failed to remove {}: {cleanup_error}",
                temp.display()
            )),
        };
    }
    Ok(())
}

/// Write `contents` to a new file at `path` that only the current user can read. A file left
/// at `path` by an earlier failed write is replaced; the new file is created exclusively, so a
/// symlink placed there is never followed.
fn write_private_file(path: &Path, contents: &str) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;

    #[cfg(windows)]
    crate::windows_acl::restrict_file_to_current_user(path).map_err(io::Error::other)?;
    #[cfg(not(any(unix, windows)))]
    return Err(io::Error::other(
        "secret files are unsupported on this platform",
    ));

    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

/// Restrict an existing secret file to the current user.
///
/// `what` names the file in error messages.
#[tracing::instrument(level = "debug", fields(path = %path.display(), %what))]
pub(crate) fn restrict_secret_permissions(path: &Path, what: &str) -> Result<()> {
    if !path.is_file() {
        bail!("{what} does not exist: {}", path.display());
    }

    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).with_context(|| {
            format!("Failed to restrict {what} permissions: {}", path.display())
        })?;
    }

    #[cfg(windows)]
    {
        crate::windows_acl::restrict_file_to_current_user(path).with_context(|| {
            format!("Failed to restrict {what} permissions: {}", path.display())
        })?;
    }

    #[cfg(not(any(unix, windows)))]
    {
        bail!("{what} permissions are unsupported on this platform");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::{SSH_PRIVATE_KEY_ERROR_LABEL, restrict_secret_permissions, write_secret_file};

    #[cfg(unix)]
    #[test]
    fn ssh_keys_are_written_owner_only() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let key_path = temp_dir.path().join("sherpa_ssh_key");

        write_secret_file(&key_path, "private key", SSH_PRIVATE_KEY_ERROR_LABEL)
            .expect("write private key");

        let mode = fs::metadata(&key_path)
            .expect("read private key metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn restrict_secret_permissions_repairs_existing_file() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let key_path = temp_dir.path().join("sherpa_ssh_key");
        fs::write(&key_path, "private key").expect("write private key");
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644))
            .expect("set permissive mode");

        restrict_secret_permissions(&key_path, SSH_PRIVATE_KEY_ERROR_LABEL)
            .expect("restrict private key");

        let mode = fs::metadata(&key_path)
            .expect("read private key metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn restrict_secret_permissions_rejects_missing_file() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let key_path = temp_dir.path().join("missing_key");

        let error = restrict_secret_permissions(&key_path, SSH_PRIVATE_KEY_ERROR_LABEL)
            .expect_err("missing private key should fail");

        assert!(error.to_string().contains("SSH private key"));
    }

    #[cfg(windows)]
    #[test]
    fn ssh_keys_are_restricted_to_the_current_user_acl() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let key_path = temp_dir.path().join("sherpa_ssh_key");

        write_secret_file(&key_path, "private key", SSH_PRIVATE_KEY_ERROR_LABEL)
            .expect("write private key");

        assert!(
            crate::windows_acl::acl_inspection::is_restricted_to_current_user(&key_path)
                .expect("inspect private key ACL")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_secret_replaces_a_symlink_instead_of_writing_through_it() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let elsewhere = temp_dir.path().join("elsewhere");
        fs::write(&elsewhere, "untouched").expect("write target");
        let path = temp_dir.path().join("kubeconfig-k8s");
        std::os::unix::fs::symlink(&elsewhere, &path).expect("create symlink");

        write_secret_file(&path, "config", "kubeconfig").expect("write secret file");

        assert_eq!(
            fs::read_to_string(&elsewhere).expect("read target"),
            "untouched"
        );
        assert!(
            !fs::symlink_metadata(&path)
                .expect("stat")
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&path).expect("read secret"), "config");
    }

    #[cfg(unix)]
    #[test]
    fn a_secret_replaces_an_open_file_and_leaves_no_temporary_file() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let path = temp_dir.path().join("kubeconfig-k8s");
        fs::write(&path, "old").expect("write old file");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("open up file");

        write_secret_file(&path, "new", "kubeconfig").expect("write secret file");

        assert_eq!(fs::read_to_string(&path).expect("read secret"), "new");
        let mode = fs::metadata(&path)
            .expect("read metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let names: Vec<_> = fs::read_dir(temp_dir.path())
            .expect("list directory")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("kubeconfig-k8s")]);
    }

    #[cfg(unix)]
    #[test]
    fn secret_files_are_owner_only_and_named_in_errors() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let path = temp_dir.path().join("kubeconfig-k8s");
        write_secret_file(&path, "config", "kubeconfig").expect("write secret file");
        let mode = fs::metadata(&path)
            .expect("read metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        let missing = temp_dir.path().join("missing").join("kubeconfig");
        let error = write_secret_file(&missing, "config", "kubeconfig")
            .expect_err("missing directory should fail");
        assert!(error.to_string().contains("Failed to write kubeconfig"));
    }
}
