use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

#[cfg(feature = "netinfo")]
use anyhow::anyhow;
use anyhow::{Context, Result};
#[cfg(feature = "netinfo")]
use getifaddrs::{Address, Interfaces, getifaddrs};
use ipnet::{Ipv4Net, Ipv6Net};
#[cfg(feature = "netinfo")]
use ipnet::{ipv4_mask_to_prefix, ipv6_mask_to_prefix};

/// Get the nth IPv4 addr from a network block given an index
pub fn get_ipv4_addr(network: &Ipv4Net, nth: u32) -> Result<Ipv4Addr> {
    let net_bits = network.network().to_bits();
    Ok(Ipv4Addr::from_bits(net_bits + nth))
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
    Ipv4Net::from_str(ipv4_net).with_context(|| "Failed to parse network: {ipv4_net}")
}

/// Get a free subnet from the supernet block that is not currently in use.
#[cfg(feature = "netinfo")]
pub fn get_free_subnet(prefix: &String) -> Result<Ipv4Net> {
    // get existing ip assigned to interfaces
    let interface_networks = get_interface_networks()?;

    // subnet, supernet block into /24's
    let supernet = Ipv4Net::from_str(prefix)?;
    let subnets: Vec<Ipv4Net> = supernet.subnets(24)?.collect();

    for subnet in subnets {
        if !ipv4_overlaps_any(&subnet, &interface_networks) {
            return Ok(subnet);
        }
    }
    Err(anyhow!("No free subnet found in supernet: {}", prefix))
}

/// Whether `subnet` overlaps any of `networks`: either contains the other's network address.
pub fn ipv4_overlaps_any(subnet: &Ipv4Net, networks: &[Ipv4Net]) -> bool {
    for network in networks {
        if subnet.contains(&network.network()) || network.contains(&subnet.network()) {
            return true;
        }
    }
    false
}

/// Whether `subnet` overlaps any of `networks`: either contains the other's network address.
pub fn ipv6_overlaps_any(subnet: &Ipv6Net, networks: &[Ipv6Net]) -> bool {
    for network in networks {
        if subnet.contains(&network.network()) || network.contains(&subnet.network()) {
            return true;
        }
    }
    false
}

/// Get the IPv6 networks currently assigned to interfaces.
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
                interface_networks.push(subnet)
            }
        }
    }
    Ok(interface_networks)
}

/// Get an IPv4 address from a loopback subnet and host address.
///
/// Combines the subnet's network address with the host address offset
/// to produce a unique loopback IP per lab per node.
pub fn get_ip(loopback_subnet: &Ipv4Net, host_addr: u8) -> Ipv4Addr {
    let net_bits = loopback_subnet.network().to_bits();
    Ipv4Addr::from_bits(net_bits + host_addr as u32)
}

/// Allocate the next free loopback `/24` subnet from the supernet prefix.
///
/// Skips `x.x.0.0/24` and returns the first `/24` not present in `used`.
pub fn allocate_loopback_subnet(prefix: &Ipv4Net, used: &[Ipv4Net]) -> Result<Ipv4Net> {
    let subnets: Vec<Ipv4Net> = prefix
        .subnets(24)
        .context("Failed to subnet loopback prefix into /24s")?
        .collect();

    for subnet in subnets {
        // Skip the x.x.0.0/24 subnet (network zero)
        if subnet.network().octets()[2] == 0 {
            continue;
        }
        if !used.contains(&subnet) {
            return Ok(subnet);
        }
    }
    Err(anyhow::anyhow!(
        "No free loopback /24 subnet found in prefix: {}",
        prefix
    ))
}

/// Allocate the next free management `/24` subnet from the supernet prefix.
///
/// Skips `x.x.0.0/24` and returns the first `/24` not present in `used` (allocated to labs)
/// and not overlapping `host` (networks already on host interfaces, such as a leftover
/// lab bridge or another service).
pub fn allocate_management_subnet(
    prefix: &Ipv4Net,
    used: &[Ipv4Net],
    host: &[Ipv4Net],
) -> Result<Ipv4Net> {
    let subnets: Vec<Ipv4Net> = prefix
        .subnets(24)
        .context("Failed to subnet management prefix into /24s")?
        .collect();

    for subnet in subnets {
        // Skip the x.x.0.0/24 subnet (network zero)
        if subnet.network().octets()[2] == 0 {
            continue;
        }
        if !used.contains(&subnet) && !ipv4_overlaps_any(&subnet, host) {
            return Ok(subnet);
        }
    }
    Err(anyhow::anyhow!(
        "No free management /24 subnet found in prefix: {}",
        prefix
    ))
}

/// Get the nth IPv6 addr from a network block given an index.
pub fn get_ipv6_addr(network: &Ipv6Net, nth: u32) -> Result<Ipv6Addr> {
    let net_bits = network.network().to_bits();
    Ok(Ipv6Addr::from_bits(net_bits + nth as u128))
}

/// Parses a CIDR notation string into an `Ipv6Net`.
pub fn get_ipv6_network(ipv6_net: &str) -> Result<Ipv6Net> {
    Ipv6Net::from_str(ipv6_net).with_context(|| format!("Failed to parse IPv6 network: {ipv6_net}"))
}

/// Allocate the next free IPv6 management `/64` subnet from the supernet prefix.
///
/// Skips `::0/64` and returns the first `/64` not present in `used` and not overlapping
/// `host` (networks already on host interfaces).
pub fn allocate_ipv6_management_subnet(
    prefix: &Ipv6Net,
    used: &[Ipv6Net],
    host: &[Ipv6Net],
) -> Result<Ipv6Net> {
    let subnets: Vec<Ipv6Net> = prefix
        .subnets(64)
        .context("Failed to subnet IPv6 management prefix into /64s")?
        .collect();

    for subnet in subnets {
        // Skip the first /64 (network zero)
        if subnet.network() == prefix.network() {
            continue;
        }
        if !used.contains(&subnet) && !ipv6_overlaps_any(&subnet, host) {
            return Ok(subnet);
        }
    }
    Err(anyhow::anyhow!(
        "No free IPv6 management /64 subnet found in prefix: {}",
        prefix
    ))
}

/// Allocate the next free IPv6 loopback `/64` subnet from the supernet prefix.
///
/// Skips the first `/64` and returns the first `/64` not present in `used`.
pub fn allocate_ipv6_loopback_subnet(prefix: &Ipv6Net, used: &[Ipv6Net]) -> Result<Ipv6Net> {
    let subnets: Vec<Ipv6Net> = prefix
        .subnets(64)
        .context("Failed to subnet IPv6 loopback prefix into /64s")?
        .collect();

    for subnet in subnets {
        // Skip the first /64 (network zero)
        if subnet.network() == prefix.network() {
            continue;
        }
        if !used.contains(&subnet) {
            return Ok(subnet);
        }
    }
    Err(anyhow::anyhow!(
        "No free IPv6 loopback /64 subnet found in prefix: {}",
        prefix
    ))
}

/// Get an IPv6 address from a subnet and host address offset.
///
/// Combines the subnet's network address with the host address offset
/// to produce a unique IPv6 per lab per node.
pub fn get_ipv6_ip(subnet: &Ipv6Net, host_addr: u16) -> Ipv6Addr {
    let net_bits = subnet.network().to_bits();
    Ipv6Addr::from_bits(net_bits + host_addr as u128)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let addr = Ipv6Addr::from_bits(base + ((index as u128) << 64));
        Ipv6Net::new(addr, 64).unwrap()
    }

    fn ipv6_loop_subnet(index: u16) -> Ipv6Net {
        let base = Ipv6Addr::from_str("fd00:1001::").unwrap().to_bits();
        let addr = Ipv6Addr::from_bits(base + ((index as u128) << 64));
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
    fn test_get_ipv6_ip() {
        let subnet = ipv6_mgmt_subnet(1);
        let ip = get_ipv6_ip(&subnet, 42);
        let expected_bits = subnet.network().to_bits() + 42u128;
        assert_eq!(ip, Ipv6Addr::from_bits(expected_bits));
    }

    #[test]
    fn test_get_ipv6_ip_zero() {
        let subnet = ipv6_mgmt_subnet(1);
        let ip = get_ipv6_ip(&subnet, 0);
        assert_eq!(ip, subnet.network());
    }

    #[test]
    fn test_get_ipv6_ip_different_subnets() {
        let subnet1 = ipv6_mgmt_subnet(1);
        let subnet2 = ipv6_mgmt_subnet(2);
        let ip1 = get_ipv6_ip(&subnet1, 1);
        let ip2 = get_ipv6_ip(&subnet2, 1);
        assert_ne!(ip1, ip2);
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
}
