use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use tracing::instrument;

/// Only bounded header metadata is read; this is not a full image integrity check.
#[derive(Debug)]
pub struct Qcow2Metadata {
    pub capacity: u64,
    pub backing_file: Option<PathBuf>,
    encrypted: bool,
    external_data: bool,
    incompatible_features: u64,
}

impl Qcow2Metadata {
    /// Check whether the header describes a supported standalone overlay base
    /// and return the requested overlay capacity in bytes.
    ///
    /// An omitted size inherits the base capacity. Explicit sizes are in GiB
    /// and may equal or exceed the base capacity. This check never modifies the
    /// base and does not validate its data or allocation tables.
    #[instrument(level = "debug")]
    pub fn overlay_capacity(&self, size_gb: Option<u16>) -> Result<u64> {
        ensure!(
            self.backing_file.is_none(),
            "Overlay base already has a backing image"
        );
        ensure!(!self.encrypted, "Encrypted QCOW2 bases are unsupported");
        ensure!(
            !self.external_data,
            "QCOW2 external data files are unsupported"
        );
        // Compression and extended L2 entries are understood by QEMU. Dirty,
        // corrupt, external-data and unknown incompatible features are rejected.
        ensure!(
            self.incompatible_features & !0x18 == 0,
            "Unsupported or unsafe QCOW2 incompatible features"
        );
        let capacity = size_gb
            .map(|size| u64::from(size) * 1024 * 1024 * 1024)
            .unwrap_or(self.capacity);
        ensure!(
            capacity >= self.capacity,
            "Requested overlay size cannot shrink base capacity ({} bytes)",
            self.capacity
        );
        Ok(capacity)
    }
}

/// Read the first QCOW2 cluster (at most 2 MiB), including dependency metadata.
///
/// Returns virtual capacity, the stored backing path, and feature flags used by
/// overlay eligibility checks. File and parsing errors include the image path;
/// guest data and allocation tables are not read.
#[instrument(level = "debug", skip(path))]
pub fn read_qcow2_metadata(path: &Path) -> Result<Qcow2Metadata> {
    let mut file = File::open(path)
        .with_context(|| format!("Cannot read QCOW2 metadata: {}", path.display()))?;
    read_metadata(&mut file).with_context(|| format!("Invalid QCOW2 metadata: {}", path.display()))
}

/// Parse a QCOW2 v2 or v3 header from a reader positioned at the image's start.
///
/// Bound memory allocation and metadata offsets to the first cluster, rejecting
/// truncated headers, invalid backing paths, and malformed extensions. Preserve
/// backing paths as stored; dependency scanning resolves relative paths later.
/// Eligibility is checked separately so existing overlays can be inspected even
/// when they have a backing file or dirty feature flags.
fn read_metadata(file: &mut (impl Read + Seek)) -> Result<Qcow2Metadata> {
    let mut header = [0_u8; 104];
    file.read_exact(&mut header[..72])?;
    ensure!(&header[..4] == b"QFI\xfb", "Image is not QCOW2");
    let version = be32(&header[4..8])?;
    ensure!(
        matches!(version, 2 | 3),
        "Unsupported QCOW2 version {version}"
    );
    let bits = be32(&header[20..24])?;
    // Check the shift before allocating a buffer controlled by image metadata.
    ensure!((9..=21).contains(&bits), "Invalid QCOW2 cluster size");
    let cluster_size = 1_usize << bits;
    let (header_length, incompatible_features, autoclear) = if version == 3 {
        file.read_exact(&mut header[72..])?;
        let length = be32(&header[100..104])? as usize;
        ensure!(
            length >= 104 && length.is_multiple_of(8),
            "Invalid QCOW2 header length"
        );
        (length, be64(&header[72..80])?, be64(&header[88..96])?)
    } else {
        (72, 0, 0)
    };
    ensure!(
        header_length <= cluster_size,
        "QCOW2 header exceeds first cluster"
    );
    let capacity = be64(&header[24..32])?;
    ensure!(capacity > 0, "QCOW2 capacity must be positive");
    let offset = be64(&header[8..16])?;
    let length = be32(&header[16..20])? as usize;
    let mut cluster = vec![0_u8; cluster_size];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut cluster)?;
    let backing_file = if offset == 0 {
        ensure!(length == 0, "QCOW2 backing length without an offset");
        None
    } else {
        ensure!(
            length > 0 && length <= 1023,
            "Invalid QCOW2 backing filename length"
        );
        // Validate the offset before subtraction and conversion to usize, so
        // hostile offsets cannot overflow arithmetic or index outside the buffer.
        ensure!(
            offset >= header_length as u64
                && offset <= cluster_size as u64
                && length as u64 <= cluster_size as u64 - offset,
            "QCOW2 backing filename outside first cluster"
        );
        let name = std::str::from_utf8(&cluster[offset as usize..offset as usize + length])?;
        ensure!(!name.contains('\0'), "QCOW2 backing filename contains NUL");
        Some(PathBuf::from(name))
    };
    let extension_end = if offset == 0 {
        cluster_size
    } else {
        offset as usize
    };
    let mut encrypted = be32(&header[32..36])? != 0;
    let mut external_data = incompatible_features & 4 != 0 || autoclear & 2 != 0;
    let mut cursor = header_length;
    // Extensions stop before the backing filename, or at the cluster boundary
    // when there is no backing file. Each payload is padded to eight bytes.
    loop {
        ensure!(
            cursor + 8 <= extension_end,
            "Unterminated QCOW2 header extensions"
        );
        let kind = be32(&cluster[cursor..cursor + 4])?;
        let size = be32(&cluster[cursor + 4..cursor + 8])? as usize;
        if kind == 0 {
            break;
        }
        let end = cursor + 8 + size;
        ensure!(
            end <= extension_end,
            "QCOW2 extension outside first cluster"
        );
        encrypted |= kind == 0x0537be77;
        external_data |= kind == 0x44415441;
        cursor = (end + 7) & !7;
    }
    Ok(Qcow2Metadata {
        capacity,
        backing_file,
        encrypted,
        external_data,
        incompatible_features,
    })
}

/// Decode an exactly four-byte, big-endian QCOW2 field; reject other lengths.
fn be32(bytes: &[u8]) -> Result<u32> {
    Ok(u32::from_be_bytes(bytes.try_into()?))
}
/// Decode an exactly eight-byte, big-endian QCOW2 field; reject other lengths.
fn be64(bytes: &[u8]) -> Result<u64> {
    Ok(u64::from_be_bytes(bytes.try_into()?))
}

/// Protect bases using actual managed disks, including stopped and orphaned volumes.
/// Missing, malformed or unreadable dependency metadata fails closed.
///
/// Canonicalize the image version directory before checking backing references.
/// The caller must hold the image lifecycle lock through this check and deletion
/// to prevent concurrent provisioning from introducing a new dependency.
#[instrument(level = "debug", skip(storage_path, image_dir))]
pub fn ensure_no_overlay_dependencies(storage_path: &Path, image_dir: &Path) -> Result<()> {
    let image_dir = fs::canonicalize(image_dir)
        .context("Cannot resolve image directory for dependency check")?;
    scan_dependencies(storage_path, &image_dir)
}

/// Recursively inspect storage files for backing references into `image_dir`.
///
/// Treat either a QCOW2 filename extension or magic signature as a reason to
/// parse metadata. Resolve relative backing paths against the overlay directory
/// and canonicalize them before comparison. Propagate inspection failures so
/// deletion cannot proceed when a dependency cannot be determined safely.
fn scan_dependencies(storage_path: &Path, image_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(storage_path)
        .context("Cannot enumerate managed storage for dependency check")?
    {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            scan_dependencies(&path, image_dir)?;
            continue;
        }
        let mut file = File::open(&path)
            .with_context(|| format!("Cannot inspect dependency metadata: {}", path.display()))?;
        let mut magic = [0_u8; 4];
        let count = file.read(&mut magic)?;
        let qcow_extension = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("qcow2"));
        if !qcow_extension && (count != 4 || &magic != b"QFI\xfb") {
            continue;
        }
        file.seek(SeekFrom::Start(0))?;
        let metadata = read_metadata(&mut file)
            .with_context(|| format!("Cannot inspect dependency metadata: {}", path.display()))?;
        if let Some(backing) = metadata.backing_file {
            let backing = if backing.is_absolute() {
                backing
            } else {
                path.parent()
                    .context("Missing disk parent directory")?
                    .join(backing)
            };
            let backing = fs::canonicalize(&backing)
                .with_context(|| format!("Cannot resolve backing image of {}", path.display()))?;
            ensure!(
                !backing.starts_with(image_dir),
                "Image deletion blocked: managed overlay {} references {}",
                path.display(),
                backing.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Build a metadata-only fixture with a 512-byte cluster and 2 GiB capacity.
    fn header(version: u32) -> Vec<u8> {
        let mut bytes = vec![0; 512];
        bytes[..4].copy_from_slice(b"QFI\xfb");
        bytes[4..8].copy_from_slice(&version.to_be_bytes());
        bytes[20..24].copy_from_slice(&9_u32.to_be_bytes());
        bytes[24..32].copy_from_slice(&(2_u64 << 30).to_be_bytes());
        bytes[100..104].copy_from_slice(&104_u32.to_be_bytes());
        bytes
    }

    #[test]
    // Supported header versions inherit capacity and permit only equal or larger sizes.
    fn standalone_versions_and_overlay_sizes() {
        for version in [2, 3] {
            let info = read_metadata(&mut Cursor::new(header(version))).unwrap();
            assert_eq!(info.overlay_capacity(None).unwrap(), 2 << 30);
            assert_eq!(info.overlay_capacity(Some(2)).unwrap(), 2 << 30);
            assert_eq!(info.overlay_capacity(Some(3)).unwrap(), 3 << 30);
            assert!(info.overlay_capacity(Some(1)).is_err());
            assert!(info.overlay_capacity(Some(0)).is_err());
        }
    }

    #[test]
    // Reject unsafe bases whether features are advertised by flags or extensions.
    fn reject_encryption_external_files_and_unsafe_features() {
        for (offset, value) in [(35, 1), (79, 1), (79, 2), (79, 4), (79, 32), (95, 2)] {
            let mut bytes = header(3);
            bytes[offset] = value;
            let info = read_metadata(&mut Cursor::new(bytes)).unwrap();
            assert!(info.overlay_capacity(None).is_err());
        }
        for kind in [0x44415441_u32, 0x0537be77] {
            let mut bytes = header(3);
            bytes[104..108].copy_from_slice(&kind.to_be_bytes());
            let info = read_metadata(&mut Cursor::new(bytes)).unwrap();
            assert!(info.overlay_capacity(None).is_err());
        }
    }

    #[test]
    // Dependency inspection must retain backing paths while base eligibility rejects them.
    fn backing_path_is_read_but_nested_bases_are_rejected() {
        let mut bytes = header(3);
        bytes[8..16].copy_from_slice(&128_u64.to_be_bytes());
        bytes[16..20].copy_from_slice(&9_u32.to_be_bytes());
        bytes[128..137].copy_from_slice(b"base.qcow");
        let info = read_metadata(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(info.backing_file, Some(PathBuf::from("base.qcow")));
        assert!(info.overlay_capacity(None).is_err());
    }

    #[test]
    // Out-of-range offsets must return errors rather than panic on integer overflow.
    fn malicious_backing_offsets_fail_without_overflow() {
        for offset in [u64::MAX, u64::MAX - 1, 513, 1] {
            let mut bytes = header(3);
            bytes[8..16].copy_from_slice(&offset.to_be_bytes());
            bytes[16..20].copy_from_slice(&1023_u32.to_be_bytes());
            assert!(read_metadata(&mut Cursor::new(bytes)).is_err());
        }
    }

    #[test]
    // Truncated or inconsistent metadata must not be accepted during dependency checks.
    fn malformed_metadata_fails_closed() {
        let original = header(3);
        for len in [0, 4, 71, 103, 511] {
            assert!(read_metadata(&mut Cursor::new(&original[..len])).is_err());
        }
        for (offset, bytes) in [(4, 4_u32), (20, 22), (100, 512)] {
            let mut damaged = header(3);
            damaged[offset..offset + 4].copy_from_slice(&bytes.to_be_bytes());
            assert!(read_metadata(&mut Cursor::new(damaged)).is_err());
        }
        let mut damaged = header(3);
        damaged[108..112].copy_from_slice(&u32::MAX.to_be_bytes());
        damaged[104] = 1;
        assert!(read_metadata(&mut Cursor::new(damaged)).is_err());
    }
}
