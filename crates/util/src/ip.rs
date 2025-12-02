use std::net::Ipv4Addr;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use getifaddrs::{Address, Interfaces, getifaddrs};
use ipnet::{Ipv4Net, ipv4_mask_to_prefix};

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
pub fn get_free_subnet(prefix: &String) -> Result<Ipv4Net> {
    // get existing ip assigned to interfaces
    let interface_networks = get_interface_networks()?;

    // subnet, supernet block into /24's
    let supernet = Ipv4Net::from_str(prefix)?;
    let subnets: Vec<Ipv4Net> = supernet.subnets(24)?.collect();

    for subnet in subnets {
        let overlaps = interface_networks.iter().any(|interface_net| {
            // Check if networks overlap by seeing if either contains the other's network address
            subnet.contains(&interface_net.network()) || interface_net.contains(&subnet.network())
        });

        if !overlaps {
            return Ok(subnet);
        }
    }
    Err(anyhow!("No free subnet found in supernet: {}", prefix))
}

/// Get a list of IP addresses currently assigned to intefaces
pub fn get_interface_networks() -> Result<Vec<Ipv4Net>> {
    let interfaces = getifaddrs().unwrap().collect::<Interfaces>();

    let mut interface_networks = vec![];
    for (_index, interface) in interfaces {
        for address in interface.address.iter().flatten() {
            if let Address::V4(..) = address
                && address.ip_addr().is_some()
                && address.netmask().is_some()
            {
                let ip_address = address.ip_addr().unwrap(); // Must be some.
                let ip = Ipv4Addr::from_str(&ip_address.to_string())?;

                // First convert to IpAddr to compute the prefix length
                let netmask_addr = address.netmask().unwrap(); // Must be some.
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

/// Get an IPv4 address from a host address.
pub fn get_ip(host_addr: u8) -> Ipv4Addr {
    Ipv4Addr::new(127, 127, 127, host_addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_get_ip_valid_host_addr() {
        let host_addr: u8 = 42;
        let expected = Ipv4Addr::new(127, 127, 127, 42);
        assert_eq!(get_ip(host_addr), expected);
    }

    #[test]
    fn test_get_ip_zero() {
        let host_addr: u8 = 0;
        let expected = Ipv4Addr::new(127, 127, 127, 0);
        assert_eq!(get_ip(host_addr), expected);
    }

    #[test]
    fn test_get_ip_max() {
        let host_addr: u8 = 255;
        let expected = Ipv4Addr::new(127, 127, 127, 255);
        assert_eq!(get_ip(host_addr), expected);
    }

    #[test]
    fn test_get_ip_first_three_octets() {
        let host_addr: u8 = 1;
        let ip = get_ip(host_addr);
        assert_eq!(ip.octets()[0..3], [127, 127, 127]);
    }

    #[test]
    fn test_get_ip_different_inputs() {
        let ip1 = get_ip(1);
        let ip2 = get_ip(2);
        assert_ne!(ip1, ip2);
    }

    #[test]
    fn test_get_ip_to_string() {
        let host_addr: u8 = 100;
        let ip = get_ip(host_addr);
        assert_eq!(ip.to_string(), "127.127.127.100");
    }
}
