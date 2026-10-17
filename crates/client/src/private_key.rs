use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// What the files written by `write_private_key` are called in messages.
const SSH_PRIVATE_KEY: &str = "SSH private key";

/// Write an SSH private key and restrict it to the current user.
#[tracing::instrument(level = "debug", skip(contents), fields(path = %path.display()))]
pub(crate) fn write_private_key(path: &Path, contents: &str) -> Result<()> {
    write_secret_file(path, contents, SSH_PRIVATE_KEY)
}

/// Write a file holding a secret, such as a kubeconfig, and restrict it to the current user.
///
/// `what` names the file in error messages.
#[tracing::instrument(level = "debug", skip(contents), fields(path = %path.display(), %what))]
pub(crate) fn write_secret_file(path: &Path, contents: &str, what: &str) -> Result<()> {
    write_secret_with_restriction(path, contents, what, restrict_secret_permissions)
}

fn write_secret_with_restriction<F>(
    path: &Path,
    contents: &str,
    what: &str,
    restrict: F,
) -> Result<()>
where
    F: FnOnce(&Path, &str) -> Result<()>,
{
    fs::write(path, contents)
        .with_context(|| format!("Failed to write {what}: {}", path.display()))?;

    if let Err(permission_error) = restrict(path, what) {
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

/// Restrict an existing SSH private key to the current user.
#[tracing::instrument(level = "debug", fields(path = %path.display()))]
pub(crate) fn restrict_private_key_permissions(path: &Path) -> Result<()> {
    restrict_secret_permissions(path, SSH_PRIVATE_KEY)
}

/// Restrict an existing secret file to the current user.
fn restrict_secret_permissions(path: &Path, what: &str) -> Result<()> {
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

    use anyhow::bail;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::{
        SSH_PRIVATE_KEY, restrict_private_key_permissions, write_private_key, write_secret_file,
        write_secret_with_restriction,
    };

    #[cfg(unix)]
    #[test]
    fn write_private_key_sets_owner_only_permissions() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let key_path = temp_dir.path().join("sherpa_ssh_key");

        write_private_key(&key_path, "private key").expect("write private key");

        let mode = fs::metadata(&key_path)
            .expect("read private key metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn restrict_private_key_permissions_repairs_existing_file() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let key_path = temp_dir.path().join("sherpa_ssh_key");
        fs::write(&key_path, "private key").expect("write private key");
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644))
            .expect("set permissive mode");

        restrict_private_key_permissions(&key_path).expect("restrict private key");

        let mode = fs::metadata(&key_path)
            .expect("read private key metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn restrict_private_key_permissions_rejects_missing_file() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let key_path = temp_dir.path().join("missing_key");

        let error = restrict_private_key_permissions(&key_path)
            .expect_err("missing private key should fail");

        assert!(error.to_string().contains("SSH private key"));
    }

    #[test]
    fn write_private_key_removes_file_when_restriction_fails() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let key_path = temp_dir.path().join("sherpa_ssh_key");

        let error =
            write_secret_with_restriction(&key_path, "private key", SSH_PRIVATE_KEY, |_, _| {
                bail!("permission failure")
            })
            .expect_err("permission failure should fail the write");

        assert!(error.to_string().contains("Removed SSH private key"));
        assert!(!key_path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn write_private_key_sets_current_user_only_acl() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let key_path = temp_dir.path().join("sherpa_ssh_key");

        write_private_key(&key_path, "private key").expect("write private key");

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
