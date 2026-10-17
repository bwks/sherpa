use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Name of an SSH private key in error messages, e.g. "Failed to write SSH private key: <path>".
pub(crate) const SSH_PRIVATE_KEY_ERROR_LABEL: &str = "SSH private key";

/// Write a file holding a secret, such as an SSH private key or a kubeconfig, and restrict it
/// to the current user. The file is removed if its permissions cannot be secured.
///
/// `what` names the file in error messages.
#[tracing::instrument(level = "debug", skip(contents), fields(path = %path.display(), %what))]
pub(crate) fn write_secret_file(path: &Path, contents: &str, what: &str) -> Result<()> {
    fs::write(path, contents)
        .with_context(|| format!("Failed to write {what}: {}", path.display()))?;

    if let Err(permission_error) = restrict_secret_permissions(path, what) {
        return match fs::remove_file(path) {
            Ok(()) => Err(permission_error.context(format!(
                "Removed {what} after permissions could not be secured"
            ))),
            Err(cleanup_error) => Err(anyhow!(
                "{permission_error:#}; also failed to remove insecure {what} {}: {cleanup_error}",
                path.display()
            )),
        };
    }

    Ok(())
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
            crate::windows_acl::is_restricted_to_current_user(&key_path)
                .expect("inspect private key ACL")
        );
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
