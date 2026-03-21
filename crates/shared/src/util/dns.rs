use anyhow::Result;
use ipnet::{Ipv4Net, Ipv6Net};

use super::ip::{get_ipv4_addr, get_ipv6_addr};
use crate::data::{Dns, NameServer};
use crate::konst::{BOOT_SERVER_NAME, SHERPA_DOMAIN_NAME};

pub fn default_dns(mgmt_net: &Ipv4Net) -> Result<Dns> {
    let ipv4_address = get_ipv4_addr(mgmt_net, 1)?;

    Ok(Dns {
        domain: SHERPA_DOMAIN_NAME.to_owned(),
        name_servers: vec![NameServer {
            name: BOOT_SERVER_NAME.to_owned(),
            ipv4_address,
            ipv6_address: None,
        }],
    })
}

pub fn default_dns_dual_stack(mgmt_net_v4: &Ipv4Net, mgmt_net_v6: &Ipv6Net) -> Result<Dns> {
    let ipv4_address = get_ipv4_addr(mgmt_net_v4, 1)?;
    let ipv6_address = get_ipv6_addr(mgmt_net_v6, 1)?;

    Ok(Dns {
        domain: SHERPA_DOMAIN_NAME.to_owned(),
        name_servers: vec![NameServer {
            name: BOOT_SERVER_NAME.to_owned(),
            ipv4_address,
            ipv6_address: Some(ipv6_address),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_default_dns() {
        let net: Ipv4Net = "172.20.0.0/24".parse().unwrap();
        let dns = default_dns(&net).unwrap();
        assert_eq!(dns.domain, SHERPA_DOMAIN_NAME);
        assert_eq!(dns.name_servers.len(), 1);
        assert_eq!(dns.name_servers[0].name, BOOT_SERVER_NAME);
        assert_eq!(
            dns.name_servers[0].ipv4_address,
            Ipv4Addr::new(172, 20, 0, 1)
        );
        assert!(dns.name_servers[0].ipv6_address.is_none());
    }

    #[test]
    fn test_default_dns_dual_stack() {
        let net_v4: Ipv4Net = "172.20.0.0/24".parse().unwrap();
        let net_v6: Ipv6Net = "fd00::/64".parse().unwrap();
        let dns = default_dns_dual_stack(&net_v4, &net_v6).unwrap();
        assert_eq!(dns.domain, SHERPA_DOMAIN_NAME);
        assert_eq!(dns.name_servers.len(), 1);
        assert_eq!(
            dns.name_servers[0].ipv4_address,
            Ipv4Addr::new(172, 20, 0, 1)
        );
        let ipv6 = dns.name_servers[0].ipv6_address.expect("has ipv6");
        assert_eq!(ipv6, "fd00::1".parse::<std::net::Ipv6Addr>().unwrap());
    }
}
