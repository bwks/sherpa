use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

/// Write a file holding a secret, such as an auth token or a kubeconfig, readable only by
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

/// Make `path` a folder only the current user can enter, creating it (its parent must exist)
/// or tightening an existing one. A symlink or a file at `path` is refused, so nothing is
/// written through a link into another folder.
#[tracing::instrument(level = "debug", fields(path = %path.display()))]
pub(crate) fn ensure_private_dir(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "{} is a symlink; Sherpa keeps lab files only in a real folder",
                path.display()
            );
        }
        Ok(metadata) if !metadata.is_dir() => {
            bail!("{} is not a folder", path.display());
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir(path).with_context(|| format!("Failed to create {}", path.display()))?;
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read {}", path.display()));
        }
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("Failed to restrict {} to its owner", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::{ensure_private_dir, restrict_secret_permissions, write_secret_file};

    /// What the secret files in these tests are called in error messages.
    const LABEL: &str = "kubeconfig";

    /// Whether only the current user can read `path`: its mode is 0600.
    #[cfg(unix)]
    fn is_owner_only(path: &Path) -> bool {
        let mode = fs::metadata(path)
            .expect("read secret file metadata")
            .permissions()
            .mode();
        mode & 0o777 == 0o600
    }

    /// Whether only the current user can read `path`: its DACL is protected from inheritance
    /// and has one entry, the current user's.
    #[cfg(windows)]
    fn is_owner_only(path: &Path) -> bool {
        crate::windows_acl::acl_inspection::is_restricted_to_current_user(path)
            .expect("inspect secret file ACL")
    }

    /// Write `contents` to a file at `path` that other users can read: mode 0644 on Unix, the
    /// ACL inherited from the folder on Windows.
    fn write_open_file(path: &Path, contents: &str) {
        fs::write(path, contents).expect("write open file");
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).expect("open up file");
        assert!(!is_owner_only(path), "the file starts readable by others");
    }

    #[test]
    fn restrict_secret_permissions_repairs_existing_file() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let path = temp_dir.path().join("kubeconfig-k8s");
        write_open_file(&path, "config");

        restrict_secret_permissions(&path, LABEL).expect("restrict secret file");

        assert!(is_owner_only(&path));
        assert_eq!(fs::read_to_string(&path).expect("read secret"), "config");
    }

    #[test]
    fn restrict_secret_permissions_rejects_missing_file() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let path = temp_dir.path().join("missing-kubeconfig");

        let error =
            restrict_secret_permissions(&path, LABEL).expect_err("missing secret file should fail");

        assert!(error.to_string().contains(LABEL));
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

    #[test]
    fn a_secret_replaces_an_open_file_and_leaves_no_temporary_file() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let path = temp_dir.path().join("kubeconfig-k8s");
        write_open_file(&path, "old");

        write_secret_file(&path, "new", "kubeconfig").expect("write secret file");

        assert_eq!(fs::read_to_string(&path).expect("read secret"), "new");
        assert!(is_owner_only(&path));
        let names: Vec<_> = fs::read_dir(temp_dir.path())
            .expect("list directory")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("kubeconfig-k8s")]);
    }

    #[test]
    fn secret_files_are_owner_only_and_named_in_errors() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let path = temp_dir.path().join("kubeconfig-k8s");
        write_secret_file(&path, "config", "kubeconfig").expect("write secret file");
        assert!(is_owner_only(&path));

        let missing = temp_dir.path().join("missing").join("kubeconfig");
        let error = write_secret_file(&missing, "config", "kubeconfig")
            .expect_err("missing directory should fail");
        assert!(error.to_string().contains("Failed to write kubeconfig"));
    }

    #[cfg(unix)]
    #[test]
    fn a_private_dir_is_created_or_tightened_to_its_owner() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let created = temp_dir.path().join("new");
        ensure_private_dir(&created).expect("create");
        let open = temp_dir.path().join("open");
        fs::create_dir(&open).expect("create open folder");
        fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).expect("open up");
        ensure_private_dir(&open).expect("tighten");
        for dir in [created, open] {
            let mode = fs::metadata(&dir).expect("stat").permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", dir.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_refused_as_a_private_dir() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let elsewhere = temp_dir.path().join("elsewhere");
        fs::create_dir(&elsewhere).expect("create target");
        fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o755)).expect("open up");
        let link = temp_dir.path().join("link");
        std::os::unix::fs::symlink(&elsewhere, &link).expect("create symlink");
        let error = ensure_private_dir(&link).expect_err("symlink refused");
        assert!(error.to_string().contains("symlink"), "{error}");
        let mode = fs::metadata(&elsewhere).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "the link's target is untouched");
    }

    #[test]
    fn a_missing_private_dir_is_created_and_a_file_is_refused() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let created = temp_dir.path().join("new");
        ensure_private_dir(&created).expect("create");
        assert!(created.is_dir());
        ensure_private_dir(&created).expect("an existing folder is kept");

        let file = temp_dir.path().join("file");
        fs::write(&file, "").expect("write file");
        let error = ensure_private_dir(&file).expect_err("file refused");
        assert!(error.to_string().contains("is not a folder"), "{error}");
    }
}
