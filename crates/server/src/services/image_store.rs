//! Placing a VM or unikernel disk image in the images directory so that it is never seen half
//! written.
//!
//! An image file moves through three states, each a type:
//!
//! - [`StagedDisk`]: bytes are written to `<file>.partial` next to the destination. Dropping it
//!   removes the partial file, so an error or a cancelled request leaves nothing behind.
//! - [`PlacedDisk`]: the complete file was renamed onto the destination, which on one filesystem
//!   is atomic: the destination is either the old file, absent, or the whole new one.
//! - recorded: the caller wrote the `node_image` row. If that fails, [`PlacedDisk::discard`] removes
//!   a file this import added; a file it replaced stays, as the old row still names it.
//!
//! A database row therefore always names a complete file. A complete file with no row (left by a
//! crash between the rename and the row) is what the image scan adopts.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use shared::data::NodeModel;
use shared::konst::{SHERPA_IMAGES_PATH, SHERPA_STORAGE_POOL_PATH};
use tracing::instrument;

/// Suffix of the file an image is written to before it is renamed into place.
const PARTIAL_SUFFIX: &str = ".partial";

/// Where images and the lab disks built on them live.
#[derive(Debug, Clone)]
pub(crate) struct ImagePaths {
    /// The images directory: `<images>/<model>/<version>/<file>`.
    pub images: PathBuf,
    /// The storage pool, holding lab disks that may be overlays on an image.
    pub storage_pool: PathBuf,
}

impl ImagePaths {
    /// The paths of an installed server.
    pub(crate) fn installed() -> Self {
        Self {
            images: PathBuf::from(SHERPA_IMAGES_PATH),
            storage_pool: PathBuf::from(SHERPA_STORAGE_POOL_PATH),
        }
    }

    /// The directory of one image version.
    pub(crate) fn version_dir(&self, model: NodeModel, version: &str) -> PathBuf {
        self.images.join(model.to_string()).join(version)
    }
}

/// What an import does about the file already at its destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistingDisk {
    /// Nothing is there: copy the source.
    Absent,
    /// A file of the source's size is there: keep it and skip the copy.
    Same,
    /// A file of another size is there, so it is not the source; it may be a truncated copy left
    /// by an earlier release. Replace it, unless lab overlays are built on it.
    Different,
}

impl ExistingDisk {
    /// Compare the file at the destination, if any, with the source by size.
    pub(crate) fn compare(source_len: u64, existing_len: Option<u64>) -> Self {
        match existing_len {
            None => Self::Absent,
            Some(len) if len == source_len => Self::Same,
            Some(_) => Self::Different,
        }
    }
}

/// An image being written to `<destination>.partial`. Dropped without [`place`](Self::place), it
/// removes the partial file.
#[derive(Debug)]
pub(crate) struct StagedDisk {
    partial: PathBuf,
    destination: PathBuf,
}

impl StagedDisk {
    /// Stage an image for `destination`. Its directory must exist.
    pub(crate) fn new(destination: &Path) -> Self {
        let mut partial = destination.as_os_str().to_owned();
        partial.push(PARTIAL_SUFFIX);
        Self {
            partial: PathBuf::from(partial),
            destination: destination.to_owned(),
        }
    }

    /// The file the image's bytes are written to.
    pub(crate) fn partial_path(&self) -> &Path {
        &self.partial
    }

    /// Copy `source` into the partial file, flush it to disk, and check that every byte arrived.
    /// It blocks, so async callers run it with `spawn_blocking`.
    ///
    /// # Errors
    ///
    /// Fails when the source cannot be read, the partial file cannot be written, or the copy is
    /// shorter or longer than the source.
    #[instrument(level = "debug", skip(self), fields(partial = %self.partial.display()))]
    pub(crate) fn copy_from(&self, source: &Path) -> Result<u64> {
        let source_file = File::open(source)
            .with_context(|| format!("Failed to open source image {}", source.display()))?;
        let expected = source_file
            .metadata()
            .with_context(|| format!("Failed to read the size of {}", source.display()))?
            .len();
        let partial = File::create(&self.partial)
            .with_context(|| format!("Failed to create {}", self.partial.display()))?;
        let mut writer = BufWriter::new(partial);
        let copied = io::copy(&mut BufReader::new(source_file), &mut writer)
            .with_context(|| format!("Failed to copy {}", source.display()))?;
        let partial = writer
            .into_inner()
            .map_err(io::IntoInnerError::into_error)
            .context("Failed to flush the image copy")?;
        partial
            .sync_all()
            .context("Failed to write the image copy to disk")?;
        if copied != expected {
            bail!(
                "Copied {copied} of {expected} bytes from {}; the source changed during the copy",
                source.display()
            );
        }
        Ok(copied)
    }

    /// Rename the complete partial file onto the destination and make the rename durable.
    ///
    /// # Errors
    ///
    /// Fails when the rename or the directory sync fails; the partial file is then removed.
    #[instrument(level = "debug", skip(self), fields(destination = %self.destination.display()))]
    pub(crate) fn place(self) -> Result<PlacedDisk> {
        let replaced = self.destination.exists();
        fs::rename(&self.partial, &self.destination).with_context(|| {
            format!(
                "Failed to move {} to {}",
                self.partial.display(),
                self.destination.display()
            )
        })?;
        if let Some(directory) = self.destination.parent() {
            File::open(directory)
                .and_then(|directory| directory.sync_all())
                .with_context(|| format!("Failed to sync directory {}", directory.display()))?;
        }
        Ok(PlacedDisk {
            path: self.destination.clone(),
            replaced,
        })
    }
}

impl Drop for StagedDisk {
    fn drop(&mut self) {
        match fs::remove_file(&self.partial) {
            Ok(()) => tracing::debug!(path = %self.partial.display(), "Removed incomplete image"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(path = %self.partial.display(), %error, "Failed to remove incomplete image");
            }
        }
    }
}

/// A complete image renamed into place, not yet recorded in the database.
#[derive(Debug)]
#[must_use = "an image placed but not recorded must be kept or discarded"]
pub(crate) struct PlacedDisk {
    path: PathBuf,
    /// Whether a file was at the destination before; it is gone, replaced by this one.
    replaced: bool,
}

impl PlacedDisk {
    /// Where the image is.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The row could not be written: remove the file if this import added it. A file that
    /// replaced another stays, since a row may already name it and it is complete.
    #[instrument(level = "debug", skip(self), fields(path = %self.path.display()))]
    pub(crate) fn discard(self) {
        if self.replaced {
            return;
        }
        if let Err(error) = fs::remove_file(&self.path) {
            tracing::warn!(path = %self.path.display(), %error, "Failed to remove unrecorded image");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_copied_image_appears_at_its_destination_only_when_placed() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.qcow2");
        fs::write(&source, b"image bytes").unwrap();
        let destination = dir.path().join("virtioa.qcow2");

        let staged = StagedDisk::new(&destination);
        assert_eq!(staged.copy_from(&source).unwrap(), 11);
        assert!(!destination.exists(), "visible before it was placed");

        let placed = staged.place().unwrap();
        assert_eq!(placed.path(), destination);
        assert_eq!(fs::read(&destination).unwrap(), b"image bytes");
        assert_eq!(entries(dir.path()), ["source.qcow2", "virtioa.qcow2"]);
    }

    #[test]
    fn a_copy_that_fails_leaves_no_file() {
        let dir = tempdir().unwrap();
        let destination = dir.path().join("virtioa.qcow2");

        let staged = StagedDisk::new(&destination);
        assert!(staged.copy_from(&dir.path().join("missing")).is_err());
        drop(staged);

        assert_eq!(entries(dir.path()), Vec::<String>::new());
    }

    #[test]
    fn an_abandoned_stage_removes_its_partial_file() {
        let dir = tempdir().unwrap();
        let destination = dir.path().join("virtioa.qcow2");
        let staged = StagedDisk::new(&destination);
        fs::write(staged.partial_path(), b"half an im").unwrap();

        drop(staged);

        assert_eq!(entries(dir.path()), Vec::<String>::new());
    }

    #[test]
    fn a_discarded_new_image_is_removed() {
        let dir = tempdir().unwrap();
        let destination = dir.path().join("virtioa.qcow2");
        let staged = StagedDisk::new(&destination);
        fs::write(staged.partial_path(), b"image").unwrap();

        staged.place().unwrap().discard();

        assert_eq!(entries(dir.path()), Vec::<String>::new());
    }

    #[test]
    fn a_discarded_replacement_stays() {
        let dir = tempdir().unwrap();
        let destination = dir.path().join("virtioa.qcow2");
        fs::write(&destination, b"trunc").unwrap();
        let staged = StagedDisk::new(&destination);
        fs::write(staged.partial_path(), b"whole image").unwrap();

        staged.place().unwrap().discard();

        assert_eq!(fs::read(&destination).unwrap(), b"whole image");
    }

    #[test]
    fn an_existing_disk_is_kept_only_when_it_has_the_source_size() {
        assert_eq!(ExistingDisk::compare(10, None), ExistingDisk::Absent);
        assert_eq!(ExistingDisk::compare(10, Some(10)), ExistingDisk::Same);
        assert_eq!(ExistingDisk::compare(10, Some(4)), ExistingDisk::Different);
    }
}
