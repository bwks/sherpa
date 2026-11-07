use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
#[cfg(feature = "netinfo")]
use getifaddrs::{Address, Interfaces, getifaddrs};
use ipnet::{Ipv4Net, Ipv6Net};
#[cfg(feature = "netinfo")]
use ipnet::{ipv4_mask_to_prefix, ipv6_mask_to_prefix};

use crate::konst::MANAGEMENT_NODE_HOST_OFFSETS;

/// Get the nth IPv4 addr from a network block given an index
///
/// # Errors
///
/// Fails when the network has no `nth` address.
pub fn get_ipv4_addr(network: &Ipv4Net, nth: u32) -> Result<Ipv4Addr> {
    let addr = network
        .network()
        .to_bits()
        .checked_add(nth)
        .map(Ipv4Addr::from_bits)
        .filter(|addr| network.contains(addr))
        .with_context(|| format!("{network} has no address {nth}"))?;
    Ok(addr)
}

/// Parses a CIDR notation string into an `Ipv4Network`.
///
/// # Parameters
/// - `cidr`: A string slice that holds the CIDR notation (e.g., "192.168.1.0/24").
///
/// # Returns
/// - `Result<Ipv4Net, anyhow::Error>`: The parsed `Ipv4Network` if successful,
///   or an error if the input string is not a valid CIDR notation.
///
/// # Errors
/// - Returns an error if the input string is not a valid CIDR notation.
pub fn get_ipv4_network(ipv4_net: &str) -> Result<Ipv4Net> {
    Ipv4Net::from_str(ipv4_net).with_context(|| format!("Failed to parse network: {ipv4_net}"))
}

/// Management host offset of the node with the given 1-based index, from the lab node block.
///
/// # Errors
///
/// Fails when the node index is past the end of the node address range.
pub fn node_management_host_offset(node_index: u16) -> Result<u32> {
    let offset = (MANAGEMENT_NODE_HOST_OFFSETS.start() + u32::from(node_index)).saturating_sub(1);
    if node_index == 0 || !MANAGEMENT_NODE_HOST_OFFSETS.contains(&offset) {
        bail!(
            "Node index {node_index} has no management address; a lab supports at most {} nodes",
            MANAGEMENT_NODE_HOST_OFFSETS.clone().count()
        );
    }
    Ok(offset)
}

/// Whether `subnet` overlaps any of `networks`: either contains the other's network address.
#[must_use]
pub fn ipv4_overlaps_any(subnet: &Ipv4Net, networks: &[Ipv4Net]) -> bool {
    for network in networks {
        if subnet.contains(&network.network()) || network.contains(&subnet.network()) {
            return true;
        }
    }
    false
}

/// Whether `subnet` overlaps any of `networks`: either contains the other's network address.
#[must_use]
pub fn ipv6_overlaps_any(subnet: &Ipv6Net, networks: &[Ipv6Net]) -> bool {
    for network in networks {
        if subnet.contains(&network.network()) || network.contains(&subnet.network()) {
            return true;
        }
    }
    false
}

/// Get the IPv6 networks currently assigned to interfaces.
///
/// # Errors
///
/// Fails when the host's interfaces cannot be listed.
#[cfg(feature = "netinfo")]
pub fn get_interface_networks_v6() -> Result<Vec<Ipv6Net>> {
    let interfaces = getifaddrs()
        .context("Failed to get network interfaces")?
        .collect::<Interfaces>();

    let mut interface_networks = vec![];
    for (_index, interface) in interfaces {
        for address in interface.address.iter().flatten() {
            if let Address::V6(..) = address
                && let Some(ip_address) = address.ip_addr()
                && let Some(netmask_addr) = address.netmask()
            {
                let ip = Ipv6Addr::from_str(&ip_address.to_string())?;
                let netmask = Ipv6Addr::from_str(&netmask_addr.to_string())?;
                let prefix = ipv6_mask_to_prefix(netmask)?;
                interface_networks.push(Ipv6Net::new(ip, prefix)?.trunc());
            }
        }
    }
    Ok(interface_networks)
}

/// Get a list of IP addresses currently assigned to intefaces
///
/// # Errors
///
/// Fails when the host's interfaces cannot be listed.
#[cfg(feature = "netinfo")]
pub fn get_interface_networks() -> Result<Vec<Ipv4Net>> {
    let interfaces = getifaddrs()
        .context("Failed to get network interfaces")?
        .collect::<Interfaces>();

    let mut interface_networks = vec![];
    for (_index, interface) in interfaces {
        for address in interface.address.iter().flatten() {
            if let Address::V4(..) = address
                && let Some(ip_address) = address.ip_addr()
                && let Some(netmask_addr) = address.netmask()
            {
                let ip = Ipv4Addr::from_str(&ip_address.to_string())?;

                // Convert to Ipv4Addr
                let netmask = Ipv4Addr::from_str(&netmask_addr.to_string())?;
                let prefix = ipv4_mask_to_prefix(netmask)?;

                let ip_network = Ipv4Net::new(ip, prefix)?;
                let subnet = Ipv4Net::new(ip_network.network(), prefix)?;
                interface_networks.push(subnet);
            }
        }
    }
    Ok(interface_networks)
}

/// Get an IPv4 address from a loopback subnet and host address.
///
/// Combines the subnet's network address with the host address offset
/// to produce a unique loopback IP per lab per node.
#[must_use]
pub fn get_ip(loopback_subnet: &Ipv4Net, host_addr: u8) -> Ipv4Addr {
    let net_bits = loopback_subnet.network().to_bits();
    Ipv4Addr::from_bits(net_bits + u32::from(host_addr))
}

/// Allocate the next free loopback `/24` subnet from the supernet prefix.
///
/// Skips `x.x.0.0/24` and returns the first `/24` not present in `used`.
///
/// # Errors
///
/// Fails when the prefix cannot be split into /24s or every /24 is in use.
pub fn allocate_loopback_subnet(prefix: &Ipv4Net, used: &[Ipv4Net]) -> Result<Ipv4Net> {
    for subnet in prefix
        .subnets(24)
        .context("Failed to subnet loopback prefix into /24s")?
    {
        // Skip the x.x.0.0/24 subnet (network zero)
        if subnet.network().octets()[2] == 0 {
            continue;
        }
        if !used.contains(&subnet) {
            return Ok(subnet);
        }
    }
    Err(anyhow::anyhow!(
        "No free loopback /24 subnet found in prefix: {prefix}"
    ))
}

/// Allocate the next free management `/24` subnet from the supernet prefix.
///
/// Skips `x.x.0.0/24` and returns the first `/24` not present in `used` (allocated to labs)
/// and not overlapping `host` (networks already on host interfaces, such as a leftover
/// lab bridge or another service).
///
/// # Errors
///
/// Fails when the prefix cannot be split into /24s or every /24 is in use or on the host.
pub fn allocate_management_subnet(
    prefix: &Ipv4Net,
    used: &[Ipv4Net],
    host: &[Ipv4Net],
) -> Result<Ipv4Net> {
    for subnet in prefix
        .subnets(24)
        .context("Failed to subnet management prefix into /24s")?
    {
        // Skip the x.x.0.0/24 subnet (network zero)
        if subnet.network().octets()[2] == 0 {
            continue;
        }
        if !used.contains(&subnet) && !ipv4_overlaps_any(&subnet, host) {
            return Ok(subnet);
        }
    }
    Err(anyhow::anyhow!(
        "No free management /24 subnet found in prefix: {prefix}"
    ))
}

/// Get the nth IPv6 addr from a network block given an index.
///
/// # Errors
///
/// Fails when the network has no `nth` address.
pub fn get_ipv6_addr(network: &Ipv6Net, nth: u32) -> Result<Ipv6Addr> {
    let addr = network
        .network()
        .to_bits()
        .checked_add(u128::from(nth))
        .map(Ipv6Addr::from_bits)
        .filter(|addr| network.contains(addr))
        .with_context(|| format!("{network} has no address {nth}"))?;
    Ok(addr)
}

/// Parses a CIDR notation string into an `Ipv6Net`.
///
/// # Errors
///
/// Fails when `ipv6_net` is not an IPv6 CIDR network.
pub fn get_ipv6_network(ipv6_net: &str) -> Result<Ipv6Net> {
    Ipv6Net::from_str(ipv6_net).with_context(|| format!("Failed to parse IPv6 network: {ipv6_net}"))
}

/// Allocate the next free IPv6 management `/64` subnet from the supernet prefix.
///
/// Skips `::0/64` and returns the first `/64` not present in `used` and not overlapping
/// `host` (networks already on host interfaces).
///
/// # Errors
///
/// Fails when the prefix cannot be split into /64s or every /64 is in use or on the host.
pub fn allocate_ipv6_management_subnet(
    prefix: &Ipv6Net,
    used: &[Ipv6Net],
    host: &[Ipv6Net],
) -> Result<Ipv6Net> {
    for subnet in prefix
        .subnets(64)
        .context("Failed to subnet IPv6 management prefix into /64s")?
    {
        // Skip the first /64 (network zero)
        if subnet.network() == prefix.network() {
            continue;
        }
        if !used.contains(&subnet) && !ipv6_overlaps_any(&subnet, host) {
            return Ok(subnet);
        }
    }
    Err(anyhow::anyhow!(
        "No free IPv6 management /64 subnet found in prefix: {prefix}"
    ))
}

/// Allocate the next free IPv6 loopback `/64` subnet from the supernet prefix.
///
/// Skips the first `/64` and returns the first `/64` not present in `used`.
///
/// # Errors
///
/// Fails when the prefix cannot be split into /64s or every /64 is in use.
pub fn allocate_ipv6_loopback_subnet(prefix: &Ipv6Net, used: &[Ipv6Net]) -> Result<Ipv6Net> {
    for subnet in prefix
        .subnets(64)
        .context("Failed to subnet IPv6 loopback prefix into /64s")?
    {
        // Skip the first /64 (network zero)
        if subnet.network() == prefix.network() {
            continue;
        }
        if !used.contains(&subnet) {
            return Ok(subnet);
        }
    }
    Err(anyhow::anyhow!(
        "No free IPv6 loopback /64 subnet found in prefix: {prefix}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_outside_its_network_is_refused() {
        let network: Ipv4Net = "10.0.0.0/30".parse().unwrap();
        assert_eq!(
            get_ipv4_addr(&network, 3).unwrap(),
            Ipv4Addr::new(10, 0, 0, 3)
        );
        assert!(get_ipv4_addr(&network, 4).is_err());
        let top: Ipv4Net = "255.255.255.255/32".parse().unwrap();
        assert!(get_ipv4_addr(&top, 1).is_err());

        let network6: Ipv6Net = "fd00::/126".parse().unwrap();
        assert!(get_ipv6_addr(&network6, 3).is_ok());
        assert!(get_ipv6_addr(&network6, 4).is_err());
    }

    #[test]
    fn a_network_that_does_not_parse_is_named() {
        let error = get_ipv4_network("10.0.0/24").unwrap_err();
        assert!(error.to_string().contains("10.0.0/24"), "{error}");
    }

    #[test]
    fn a_large_ipv6_prefix_is_not_listed_whole() {
        let prefix: Ipv6Net = "fd00::/32".parse().unwrap();
        let subnet = allocate_ipv6_management_subnet(&prefix, &[], &[]).unwrap();
        assert_eq!(subnet, "fd00:0:0:1::/64".parse::<Ipv6Net>().unwrap());
        let loopback = allocate_ipv6_loopback_subnet(&prefix, &[]).unwrap();
        assert_eq!(loopback, "fd00:0:0:1::/64".parse::<Ipv6Net>().unwrap());
    }
    use std::net::Ipv4Addr;

    fn test_subnet(third_octet: u8) -> Ipv4Net {
        Ipv4Net::new(Ipv4Addr::new(127, 127, third_octet, 0), 24).unwrap()
    }

    fn mgmt_subnet(third_octet: u8) -> Ipv4Net {
        Ipv4Net::new(Ipv4Addr::new(172, 31, third_octet, 0), 24).unwrap()
    }

    #[test]
    fn test_get_ip_valid_host_addr() {
        let subnet = test_subnet(1);
        let expected = Ipv4Addr::new(127, 127, 1, 42);
        assert_eq!(get_ip(&subnet, 42), expected);
    }

    #[test]
    fn test_get_ip_zero() {
        let subnet = test_subnet(1);
        let expected = Ipv4Addr::new(127, 127, 1, 0);
        assert_eq!(get_ip(&subnet, 0), expected);
    }

    #[test]
    fn test_get_ip_max() {
        let subnet = test_subnet(1);
        let expected = Ipv4Addr::new(127, 127, 1, 255);
        assert_eq!(get_ip(&subnet, 255), expected);
    }

    #[test]
    fn test_get_ip_different_subnets() {
        let subnet1 = test_subnet(1);
        let subnet2 = test_subnet(2);
        let ip1 = get_ip(&subnet1, 1);
        let ip2 = get_ip(&subnet2, 1);
        assert_ne!(ip1, ip2);
        assert_eq!(ip1.to_string(), "127.127.1.1");
        assert_eq!(ip2.to_string(), "127.127.2.1");
    }

    #[test]
    fn test_get_ip_different_hosts() {
        let subnet = test_subnet(5);
        let ip1 = get_ip(&subnet, 1);
        let ip2 = get_ip(&subnet, 2);
        assert_ne!(ip1, ip2);
    }

    #[test]
    fn test_get_ip_to_string() {
        let subnet = test_subnet(3);
        let ip = get_ip(&subnet, 100);
        assert_eq!(ip.to_string(), "127.127.3.100");
    }

    #[test]
    fn test_allocate_loopback_subnet_empty() {
        let prefix = Ipv4Net::from_str("127.127.0.0/16").unwrap();
        let result = allocate_loopback_subnet(&prefix, &[]).unwrap();
        // Should skip 127.127.0.0/24 and return 127.127.1.0/24
        assert_eq!(result, test_subnet(1));
    }

    #[test]
    fn test_allocate_loopback_subnet_skips_used() {
        let prefix = Ipv4Net::from_str("127.127.0.0/16").unwrap();
        let used = vec![test_subnet(1)];
        let result = allocate_loopback_subnet(&prefix, &used).unwrap();
        assert_eq!(result, test_subnet(2));
    }

    #[test]
    fn test_allocate_loopback_subnet_skips_multiple_used() {
        let prefix = Ipv4Net::from_str("127.127.0.0/16").unwrap();
        let used = vec![test_subnet(1), test_subnet(2), test_subnet(3)];
        let result = allocate_loopback_subnet(&prefix, &used).unwrap();
        assert_eq!(result, test_subnet(4));
    }

    #[test]
    fn test_allocate_management_subnet_empty() {
        let prefix = Ipv4Net::from_str("172.31.0.0/16").unwrap();
        let result = allocate_management_subnet(&prefix, &[], &[]).unwrap();
        // Should skip 172.31.0.0/24 and return 172.31.1.0/24
        assert_eq!(result, mgmt_subnet(1));
    }

    #[test]
    fn test_allocate_management_subnet_skips_used() {
        let prefix = Ipv4Net::from_str("172.31.0.0/16").unwrap();
        let used = vec![mgmt_subnet(1)];
        let result = allocate_management_subnet(&prefix, &used, &[]).unwrap();
        assert_eq!(result, mgmt_subnet(2));
    }

    #[test]
    fn test_allocate_management_subnet_skips_multiple_used() {
        let prefix = Ipv4Net::from_str("172.31.0.0/16").unwrap();
        let used = vec![mgmt_subnet(1), mgmt_subnet(2), mgmt_subnet(3)];
        let result = allocate_management_subnet(&prefix, &used, &[]).unwrap();
        assert_eq!(result, mgmt_subnet(4));
    }

    #[test]
    fn test_allocate_management_subnet_skips_host_networks() {
        let prefix = Ipv4Net::from_str("172.31.0.0/16").unwrap();
        // A leftover lab bridge holds 172.31.1.1/24; another range covers 172.31.2.0/23.
        let host = vec![
            Ipv4Net::from_str("172.31.1.0/24").unwrap(),
            Ipv4Net::from_str("172.31.2.0/23").unwrap(),
        ];
        let result = allocate_management_subnet(&prefix, &[], &host).unwrap();
        assert_eq!(result, mgmt_subnet(4));
    }

    #[test]
    fn test_allocate_management_subnet_fails_when_host_covers_prefix() {
        let prefix = Ipv4Net::from_str("172.31.0.0/16").unwrap();
        let host = vec![Ipv4Net::from_str("172.31.0.0/16").unwrap()];
        assert!(allocate_management_subnet(&prefix, &[], &host).is_err());
    }

    #[test]
    fn test_ipv4_overlap_is_symmetric() {
        let small = Ipv4Net::from_str("10.0.1.0/24").unwrap();
        let large = Ipv4Net::from_str("10.0.0.0/16").unwrap();
        let other = Ipv4Net::from_str("10.1.0.0/24").unwrap();
        assert!(ipv4_overlaps_any(&small, &[large]));
        assert!(ipv4_overlaps_any(&large, &[small]));
        assert!(!ipv4_overlaps_any(&small, &[other]));
    }

    // ========================================================================
    // IPv6 tests

    #[test]
    fn test_allocate_ipv6_management_subnet_skips_host_networks() {
        let prefix = Ipv6Net::from_str("fd00:b00b::/48").unwrap();
        let host = vec![
            ipv6_mgmt_subnet(1),
            Ipv6Net::from_str("fd00:b00b:0:2::/63").unwrap(),
        ];
        let result = allocate_ipv6_management_subnet(&prefix, &[], &host).unwrap();
        assert_eq!(result, ipv6_mgmt_subnet(4));
    }
    // ========================================================================

    fn ipv6_mgmt_subnet(index: u16) -> Ipv6Net {
        let base = Ipv6Addr::from_str("fd00:b00b::").unwrap().to_bits();
        let addr = Ipv6Addr::from_bits(base + (u128::from(index) << 64));
        Ipv6Net::new(addr, 64).unwrap()
    }

    fn ipv6_loop_subnet(index: u16) -> Ipv6Net {
        let base = Ipv6Addr::from_str("fd00:1001::").unwrap().to_bits();
        let addr = Ipv6Addr::from_bits(base + (u128::from(index) << 64));
        Ipv6Net::new(addr, 64).unwrap()
    }

    #[test]
    fn test_get_ipv6_addr() {
        let net = Ipv6Net::from_str("fd00:b00b:0:1::/64").unwrap();
        let addr = get_ipv6_addr(&net, 42).unwrap();
        assert_eq!(addr, Ipv6Addr::from_str("fd00:b00b:0:1::2a").unwrap());
    }

    #[test]
    fn test_get_ipv6_network() {
        let net = get_ipv6_network("fd00:b00b::/48").unwrap();
        assert_eq!(net.prefix_len(), 48);
    }

    #[test]
    fn test_get_ipv6_network_invalid() {
        assert!(get_ipv6_network("not-a-network").is_err());
    }

    #[test]
    fn test_allocate_ipv6_management_subnet_empty() {
        let prefix = Ipv6Net::from_str("fd00:b00b::/48").unwrap();
        let result = allocate_ipv6_management_subnet(&prefix, &[], &[]).unwrap();
        assert_eq!(result, ipv6_mgmt_subnet(1));
    }

    #[test]
    fn test_allocate_ipv6_management_subnet_skips_used() {
        let prefix = Ipv6Net::from_str("fd00:b00b::/48").unwrap();
        let used = vec![ipv6_mgmt_subnet(1)];
        let result = allocate_ipv6_management_subnet(&prefix, &used, &[]).unwrap();
        assert_eq!(result, ipv6_mgmt_subnet(2));
    }

    #[test]
    fn test_allocate_ipv6_management_subnet_skips_multiple_used() {
        let prefix = Ipv6Net::from_str("fd00:b00b::/48").unwrap();
        let used = vec![
            ipv6_mgmt_subnet(1),
            ipv6_mgmt_subnet(2),
            ipv6_mgmt_subnet(3),
        ];
        let result = allocate_ipv6_management_subnet(&prefix, &used, &[]).unwrap();
        assert_eq!(result, ipv6_mgmt_subnet(4));
    }

    #[test]
    fn test_allocate_ipv6_loopback_subnet_empty() {
        let prefix = Ipv6Net::from_str("fd00:1001::/48").unwrap();
        let result = allocate_ipv6_loopback_subnet(&prefix, &[]).unwrap();
        assert_eq!(result, ipv6_loop_subnet(1));
    }

    #[test]
    fn test_allocate_ipv6_loopback_subnet_skips_used() {
        let prefix = Ipv6Net::from_str("fd00:1001::/48").unwrap();
        let used = vec![ipv6_loop_subnet(1)];
        let result = allocate_ipv6_loopback_subnet(&prefix, &used).unwrap();
        assert_eq!(result, ipv6_loop_subnet(2));
    }

    #[test]
    fn test_node_management_host_offset_starts_at_node_block() {
        assert_eq!(node_management_host_offset(1).unwrap(), 21);
        assert_eq!(node_management_host_offset(2).unwrap(), 22);
        assert_eq!(node_management_host_offset(234).unwrap(), 254);
    }

    #[test]
    fn test_node_management_host_offset_rejects_out_of_range_indexes() {
        assert!(node_management_host_offset(0).is_err());
        assert!(node_management_host_offset(235).is_err());
        assert!(node_management_host_offset(u16::MAX).is_err());
    }
}
