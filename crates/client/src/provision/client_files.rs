//! Paths of the client files that provisioners read.
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use shared::util::expand_path;

/// Resolve a client path and check that it is a readable file.
pub(super) fn existing_client_file(manifest_dir: &Path, path: &str) -> Result<PathBuf> {
    let resolved = resolve_client_path(manifest_dir, path);
    let metadata = fs::metadata(&resolved)
        .with_context(|| format!("Cannot read provisioner file {}", resolved.display()))?;
    ensure!(
        metadata.is_file(),
        "Provisioner file {} is not a file",
        resolved.display()
    );
    Ok(resolved)
}

/// A client path: `~` is expanded, and relative paths start at the manifest's directory.
pub(super) fn resolve_client_path(manifest_dir: &Path, path: &str) -> PathBuf {
    let expanded = PathBuf::from(expand_path(path));
    if expanded.is_absolute() {
        expanded
    } else {
        manifest_dir.join(expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn client_paths_start_at_the_manifest_unless_absolute() {
        let dir = Path::new("/labs/dev");
        assert_eq!(
            resolve_client_path(dir, "secrets/a.env"),
            Path::new("/labs/dev/secrets/a.env")
        );
        let absolute = env::temp_dir().join("a");
        assert_eq!(
            resolve_client_path(dir, absolute.to_str().unwrap()),
            absolute
        );
        let home = expand_path("~");
        assert_eq!(
            resolve_client_path(dir, "~/a.env"),
            Path::new(&home).join("a.env")
        );
    }
}
