//! Disk sizes.

/// `gib` gibibytes in bytes. Sherpa's disk sizes are whole GiB.
pub(crate) fn gib_to_bytes(gib: u16) -> u64 {
    u64::from(gib) << 30
}
