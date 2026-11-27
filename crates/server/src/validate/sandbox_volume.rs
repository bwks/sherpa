use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use iso::{ISO_MAX_DEPTH, ISO_MAX_NAME_BYTES};
use shared::data::NodeConfig;
use shared::validators::effective_ztp;
use tokio::task;
use topology::NodeExpanded;

use crate::services::error::{SandboxRefusal, SandboxVolumeRefusal, ServiceError};
use crate::services::node_ops::manifest_volumes;
use crate::services::sandbox::DRIVE_VOLUME_DEPTH;

/// The folder levels a volume's source may have below it on a sandbox's config drive.
const MAX_VOLUME_LEVELS: usize = ISO_MAX_DEPTH - DRIVE_VOLUME_DEPTH;

/// The name of node `node` of image `image` with the source of each manifest volume it puts on
/// its sandbox's config drive, none when it runs in no sandbox.
#[must_use]
pub fn sandbox_volume_sources(node: &NodeExpanded, image: &NodeConfig) -> Vec<(String, PathBuf)> {
    if !node.model_kind.runs_in_sandbox() {
        return Vec::new();
    }
    manifest_volumes(node, effective_ztp(node.ztp.method, image))
        .iter()
        .map(|volume| (node.name.clone(), PathBuf::from(&volume.src)))
        .collect()
}

/// Refuse each of `sources`, a sandbox node's name and a volume source, that is or holds what a
/// config drive cannot carry, reading them off the async runtime and leaving a path that cannot
/// be read to the copy that reports it.
///
/// # Errors
///
/// Fails with [`SandboxRefusal::VolumeRefused`] naming the first symlink, special file,
/// folder nested too deep, file of 4 GiB or more, or name too long.
pub async fn validate_sandbox_volumes(sources: Vec<(String, PathBuf)>) -> anyhow::Result<()> {
    task::spawn_blocking(move || {
        sources
            .iter()
            .try_for_each(|(node, source)| validate_sandbox_volume(node, source))
    })
    .await
    .context("The sandbox volume check panicked")??;
    Ok(())
}

/// Refuse volume source `source` of sandbox node `node` when it is or holds what a config drive
/// cannot carry.
fn validate_sandbox_volume(node: &str, source: &Path) -> Result<(), ServiceError> {
    check_entry(node, source, 0)
}

/// Check `path`, `level` folders below its volume's source, and what it holds.
fn check_entry(node: &str, path: &Path, level: usize) -> Result<(), ServiceError> {
    let refused = |refusal| {
        ServiceError::Sandbox(SandboxRefusal::VolumeRefused {
            node: node.to_owned(),
            path: path.to_path_buf(),
            refusal,
        })
    };
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    let name_length = path
        .file_name()
        .and_then(|name| name.to_str())
        .map_or(0, |name| name.chars().count());
    if level > 0 && name_length > ISO_MAX_NAME_BYTES {
        return Err(refused(SandboxVolumeRefusal::NameTooLong {
            limit: ISO_MAX_NAME_BYTES,
        }));
    }
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(refused(SandboxVolumeRefusal::Symlink));
    }
    if file_type.is_file() {
        return match u32::try_from(metadata.len()) {
            Ok(_) => Ok(()),
            Err(_) => Err(refused(SandboxVolumeRefusal::TooLarge)),
        };
    }
    if !file_type.is_dir() {
        return Err(refused(SandboxVolumeRefusal::SpecialFile));
    }
    if level > MAX_VOLUME_LEVELS {
        return Err(refused(SandboxVolumeRefusal::TooDeep {
            limit: MAX_VOLUME_LEVELS,
        }));
    }
    let Ok(entries) = fs::read_dir(path) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        check_entry(node, &entry.path(), level + 1)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::os::unix::fs::symlink;

    use nix::sys::stat::Mode;
    use nix::unistd::mkfifo;
    use tempfile::TempDir;

    use super::*;

    /// The refusal of volume source `source` of node `n1`, or `None`.
    fn refusal(source: &Path) -> Option<(PathBuf, SandboxVolumeRefusal)> {
        match validate_sandbox_volume("n1", source) {
            Err(ServiceError::Sandbox(SandboxRefusal::VolumeRefused { path, refusal, .. })) => {
                Some((path, refusal))
            }
            Err(other) => panic!("{other}"),
            Ok(()) => None,
        }
    }

    #[test]
    fn sandbox_090_a_volume_holding_a_symlink_is_refused_naming_it() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("v");
        fs::create_dir_all(source.join("sub")).unwrap();
        fs::write(source.join("sub/a.conf"), "a").unwrap();
        symlink("a.conf", source.join("sub/link")).unwrap();
        assert_eq!(
            refusal(&source),
            Some((source.join("sub/link"), SandboxVolumeRefusal::Symlink))
        );
        let linked = dir.path().join("linked");
        symlink(&source, &linked).unwrap();
        assert_eq!(
            refusal(&linked),
            Some((linked.clone(), SandboxVolumeRefusal::Symlink))
        );
    }

    #[test]
    fn sandbox_091_a_volume_holding_a_special_file_is_refused_naming_it() {
        let dir = TempDir::new().unwrap();
        let fifo = dir.path().join("fifo");
        mkfifo(&fifo, Mode::S_IRWXU).unwrap();
        assert_eq!(
            refusal(dir.path()),
            Some((fifo, SandboxVolumeRefusal::SpecialFile))
        );
    }

    #[test]
    fn sandbox_092_a_volume_with_folders_more_than_four_levels_below_it_is_refused() {
        let dir = TempDir::new().unwrap();
        let deepest = dir.path().join("1/2/3/4");
        fs::create_dir_all(&deepest).unwrap();
        fs::write(deepest.join("a.conf"), "a").unwrap();
        assert_eq!(refusal(dir.path()), None);
        fs::create_dir(deepest.join("5")).unwrap();
        assert_eq!(
            refusal(dir.path()),
            Some((
                deepest.join("5"),
                SandboxVolumeRefusal::TooDeep { limit: 4 }
            ))
        );
    }

    #[test]
    fn sandbox_093_a_volume_holding_a_file_of_4_gib_or_more_is_refused() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("disk.img");
        let sparse = File::create(&file).unwrap();
        sparse.set_len((4 << 30) - 1).unwrap();
        assert_eq!(refusal(dir.path()), None);
        sparse.set_len(4 << 30).unwrap();
        assert_eq!(
            refusal(dir.path()),
            Some((file, SandboxVolumeRefusal::TooLarge))
        );
    }

    #[test]
    fn sandbox_094_a_volume_holding_a_name_longer_than_31_characters_is_refused() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a".repeat(31)), "a").unwrap();
        assert_eq!(refusal(dir.path()), None);
        let long = dir.path().join("a".repeat(32));
        fs::write(&long, "a").unwrap();
        assert_eq!(
            refusal(dir.path()),
            Some((long, SandboxVolumeRefusal::NameTooLong { limit: 31 }))
        );
    }
}
