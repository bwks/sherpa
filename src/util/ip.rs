use anyhow::{anyhow, Result};
use std::net::Ipv4Addr;

use ipnetwork::Ipv4Network;

/// Get an IPv4 address from a host address.
pub fn get_ip(host_addr: u8) -> Ipv4Addr {
    Ipv4Addr::new(127, 127, 127, host_addr)
}

/// Parses a CIDR notation string into an `Ipv4Network`.
///
/// # Parameters
/// - `cidr`: A string slice that holds the CIDR notation (e.g., "192.168.1.0/24").
///
/// # Returns
/// - `Result<Ipv4Network, anyhow::Error>`: The parsed `Ipv4Network` if successful,
///   or an error if the input string is not a valid CIDR notation.
///
/// # Errors
/// - Returns an error if the input string is not a valid CIDR notation.
pub fn get_ipv4_network(ipv4_net: &str) -> Result<Ipv4Network> {
    ipv4_net
        .parse::<Ipv4Network>()
        .map_err(|e| anyhow!("Failed to parse network: {}", e))
}

/// Retrieves an IPv4 address from a given network and offset.
///
/// # Parameters
/// - `network`: The base IPv4 network address.
/// - `offset`: The offset from the base network address to get the desired IPv4 address.
///
/// # Returns
/// - `Result<Ipv4Addr, anyhow::Error>`: The IPv4 address at the specified offset within the network,
///   or an error if the offset is out of range or the network is invalid.
///
/// # Errors
/// - Returns an error if the offset is out of the network range.
/// - Returns an error if the network is invalid.
pub fn get_ipv4_addr(ipv4_net: Ipv4Network, nth: u32) -> Result<Ipv4Addr> {
    ipv4_net
        .nth(nth)
        .ok_or_else(|| anyhow!("Failed to get IP:{nth} from network: {ipv4_net}"))
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
