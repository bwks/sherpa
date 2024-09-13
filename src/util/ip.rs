use std::net::Ipv4Addr;

/// Get an IPv4 address from a host address.
pub fn get_ip(host_addr: u8) -> Ipv4Addr {
    let addr = Ipv4Addr::new(127, 127, 127, host_addr);
    addr
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
