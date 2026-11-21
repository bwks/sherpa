use anyhow::{Context, Result};
use ipnet::{Ipv4Net, Ipv6Net};
use tracing::instrument;

use shared::util::{allocate_ipv6_management_subnet, allocate_management_subnet};

/// Check that a lab can be given a management IPv4 `/24` from `prefix`, by the same allocation
/// rule `up` uses, with no subnet taken yet. It runs `up`'s allocator and discards the subnet,
/// rather than restating its rules, so `sherpad init` and `up` cannot disagree about which
/// prefixes work.
#[instrument(level = "debug")]
pub fn validate_management_prefix_ipv4(prefix: Ipv4Net) -> Result<()> {
    allocate_management_subnet(&prefix, &[], &[]).with_context(|| {
        format!(
            "Management prefix {prefix} has no /24 a lab can use. It must be a /24 or shorter, \
             and the allocator skips the x.x.0.0/24"
        )
    })?;
    Ok(())
}

/// Check that a lab can be given a management IPv6 `/64` from `prefix`, by the same allocation
/// rule `up` uses, with no subnet taken yet.
#[instrument(level = "debug")]
pub fn validate_management_prefix_ipv6(prefix: Ipv6Net) -> Result<()> {
    allocate_ipv6_management_subnet(&prefix, &[], &[]).with_context(|| {
        format!(
            "IPv6 management prefix {prefix} has no /64 a lab can use. It must be a /63 or \
             shorter, since the allocator skips the prefix's first /64"
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::util::get_ipv4_network;

    #[test]
    fn lab_addressing_008_a_management_range_too_small_for_one_lab_is_refused() {
        for prefix in [
            "172.31.0.0/16",
            "172.30.0.0/16",
            "10.0.0.0/23",
            "10.0.5.0/24",
        ] {
            assert!(
                validate_management_prefix_ipv4(prefix.parse().unwrap()).is_ok(),
                "{prefix} was refused"
            );
        }

        for prefix in ["172.30.0.0/25", "10.0.0.0/24", "10.0.0.0/32"] {
            assert!(
                validate_management_prefix_ipv4(prefix.parse().unwrap()).is_err(),
                "{prefix} was accepted"
            );
        }

        for prefix in ["fd00:b00b::/48", "fd00:b00a::/48", "fd00:b00a::/63"] {
            assert!(
                validate_management_prefix_ipv6(prefix.parse().unwrap()).is_ok(),
                "{prefix} was refused"
            );
        }

        for prefix in ["fd00:b00a::/64", "fd00:b00a::/80", "fd00:b00a::/128"] {
            assert!(
                validate_management_prefix_ipv6(prefix.parse().unwrap()).is_err(),
                "{prefix} was accepted"
            );
        }

        let error = get_ipv4_network("10.0.0/24").unwrap_err();
        assert!(error.to_string().contains("10.0.0/24"), "{error}");
    }
}
