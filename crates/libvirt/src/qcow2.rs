use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use tracing::instrument;

use crate::size::gib_to_bytes;

/// The magic at the start of every QCOW2 image.
const QCOW2_MAGIC: &[u8; 4] = b"QFI\xfb";
/// Incompatible features QEMU handles in an overlay base: compression type (bit 3) and extended
/// L2 entries (bit 4).
const QCOW2_ALLOWED_INCOMPATIBLE_FEATURES: u64 = 0x18;
/// Incompatible feature bit 2: the image's data is in an external file.
const QCOW2_INCOMPATIBLE_EXTERNAL_DATA: u64 = 0x4;
/// Autoclear feature bit 1: the external data file is raw.
const QCOW2_AUTOCLEAR_RAW_EXTERNAL_DATA: u64 = 0x2;
/// Header extension holding the full disk encryption header pointer.
const QCOW2_EXTENSION_ENCRYPTION: u32 = 0x0537_be77;
/// Header extension naming the external data file.
const QCOW2_EXTENSION_DATA_FILE: u32 = 0x4441_5441;

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
    ///
    /// # Errors
    ///
    /// Fails when the base has a backing file, encryption, an external data file or an
    /// unsupported incompatible feature, or the size would shrink it.
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
            self.incompatible_features & !QCOW2_ALLOWED_INCOMPATIBLE_FEATURES == 0,
            "Unsupported or unsafe QCOW2 incompatible features"
        );
        let capacity = size_gb.map_or(self.capacity, gib_to_bytes);
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
///
/// # Errors
///
/// Fails when the file cannot be read or its header is not a valid QCOW2 v2 or v3 header.
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
    ensure!(&header[..4] == QCOW2_MAGIC, "Image is not QCOW2");
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
    // Extensions stop before the backing filename, or at the cluster boundary when there is
    // no backing file.
    let (backing_file, extension_end) = if offset == 0 {
        ensure!(length == 0, "QCOW2 backing length without an offset");
        (None, cluster_size)
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
        let start = usize::try_from(offset)?;
        let name = str::from_utf8(&cluster[start..start + length])?;
        ensure!(!name.contains('\0'), "QCOW2 backing filename contains NUL");
        (Some(PathBuf::from(name)), start)
    };
    let mut encrypted = be32(&header[32..36])? != 0;
    let mut external_data = incompatible_features & QCOW2_INCOMPATIBLE_EXTERNAL_DATA != 0
        || autoclear & QCOW2_AUTOCLEAR_RAW_EXTERNAL_DATA != 0;
    let mut cursor = header_length;
    // Each extension's payload is padded to eight bytes.
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
        encrypted |= kind == QCOW2_EXTENSION_ENCRYPTION;
        external_data |= kind == QCOW2_EXTENSION_DATA_FILE;
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

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// Build a metadata-only fixture with a 512-byte cluster and 2 GiB capacity.
    fn header(version: u32) -> Vec<u8> {
        let mut bytes = vec![0; 512];
        bytes[..4].copy_from_slice(QCOW2_MAGIC);
        bytes[4..8].copy_from_slice(&version.to_be_bytes());
        bytes[20..24].copy_from_slice(&9_u32.to_be_bytes());
        bytes[24..32].copy_from_slice(&(2_u64 << 30).to_be_bytes());
        bytes[100..104].copy_from_slice(&104_u32.to_be_bytes());
        bytes
    }

    #[test]
    // Supported header versions inherit capacity and permit only equal or larger sizes.
    fn vm_start_003_a_boot_disk_smaller_than_its_base_image_is_refused() {
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
    // Unsafe bases are refused whether their features are advertised by flags or extensions,
    // when they are overlays themselves, and when their header is damaged or truncated,
    // without panicking on integer overflow.
    fn vm_start_007_a_base_image_that_cannot_back_an_overlay_is_refused() {
        for (offset, value) in [(35, 1), (79, 1), (79, 2), (79, 4), (79, 32), (95, 2)] {
            let mut bytes = header(3);
            bytes[offset] = value;
            let info = read_metadata(&mut Cursor::new(bytes)).unwrap();
            assert!(info.overlay_capacity(None).is_err());
        }
        for kind in [QCOW2_EXTENSION_DATA_FILE, QCOW2_EXTENSION_ENCRYPTION] {
            let mut bytes = header(3);
            bytes[104..108].copy_from_slice(&kind.to_be_bytes());
            let info = read_metadata(&mut Cursor::new(bytes)).unwrap();
            assert!(info.overlay_capacity(None).is_err());
        }

        let mut bytes = header(3);
        bytes[8..16].copy_from_slice(&128_u64.to_be_bytes());
        bytes[16..20].copy_from_slice(&9_u32.to_be_bytes());
        bytes[128..137].copy_from_slice(b"base.qcow");
        let info = read_metadata(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(info.backing_file, Some(PathBuf::from("base.qcow")));
        assert!(info.overlay_capacity(None).is_err());

        for offset in [u64::MAX, u64::MAX - 1, 513, 1] {
            let mut bytes = header(3);
            bytes[8..16].copy_from_slice(&offset.to_be_bytes());
            bytes[16..20].copy_from_slice(&1023_u32.to_be_bytes());
            assert!(read_metadata(&mut Cursor::new(bytes)).is_err());
        }

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
