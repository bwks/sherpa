use anyhow::{Context, Result};
use ipnet::Ipv4Net;
use tracing::instrument;

use shared::util::allocate_management_subnet;

/// Check that a lab can be given a management `/24` from `prefix`, by the same allocation
/// rule `up` uses, with no subnet taken yet.
#[instrument(level = "debug")]
pub fn validate_management_prefix(prefix: Ipv4Net) -> Result<()> {
    allocate_management_subnet(&prefix, &[], &[]).with_context(|| {
        format!(
            "Management prefix {prefix} has no /24 a lab can use. It must be a /24 or shorter, \
             and the allocator skips the x.x.0.0/24"
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_prefixes_with_room_for_a_lab() {
        for prefix in [
            "172.31.0.0/16",
            "172.30.0.0/16",
            "10.0.0.0/23",
            "10.0.5.0/24",
        ] {
            assert!(
                validate_management_prefix(prefix.parse().unwrap()).is_ok(),
                "{prefix} was refused"
            );
        }
    }

    #[test]
    fn refuses_prefixes_without_room_for_a_lab() {
        for prefix in ["172.30.0.0/25", "10.0.0.0/24", "10.0.0.0/32"] {
            assert!(
                validate_management_prefix(prefix.parse().unwrap()).is_err(),
                "{prefix} was accepted"
            );
        }
    }
}
