//! Disk sizes.

/// `gib` gibibytes in bytes. Sherpa's disk sizes are whole GiB.
pub(crate) fn gib_to_bytes(gib: u16) -> u64 {
    u64::from(gib) << 30
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gibibytes_convert_to_bytes() {
        assert_eq!(gib_to_bytes(0), 0);
        assert_eq!(gib_to_bytes(100), 107_374_182_400);
        assert_eq!(gib_to_bytes(u16::MAX), 70_367_670_435_840);
    }
}
