use anyhow::Result;
use ipnet::{Ipv4Net, Ipv6Net};

use super::ip::{get_ipv4_addr, get_ipv6_addr};
use crate::data::{Dns, NameServer};
use crate::konst::{MANAGEMENT_GATEWAY_HOST_OFFSET, SHERPA_DOMAIN_NAME};

/// The lab's DNS settings: Sherpa's domain, with the management gateway as name server.
///
/// # Errors
///
/// Fails when the management network has no gateway address.
pub fn default_dns(mgmt_net: &Ipv4Net) -> Result<Dns> {
    let ipv4_address = get_ipv4_addr(mgmt_net, MANAGEMENT_GATEWAY_HOST_OFFSET)?;

    Ok(Dns {
        domain: SHERPA_DOMAIN_NAME.to_owned(),
        name_servers: vec![NameServer {
            ipv4_address,
            ipv6_address: None,
        }],
    })
}

/// The lab's DNS settings: Sherpa's domain, with the IPv4 and IPv6 management gateways as
/// name server.
///
/// # Errors
///
/// Fails when either management network has no gateway address.
pub fn default_dns_dual_stack(mgmt_net_v4: &Ipv4Net, mgmt_net_v6: &Ipv6Net) -> Result<Dns> {
    let ipv4_address = get_ipv4_addr(mgmt_net_v4, MANAGEMENT_GATEWAY_HOST_OFFSET)?;
    let ipv6_address = get_ipv6_addr(mgmt_net_v6, MANAGEMENT_GATEWAY_HOST_OFFSET)?;

    Ok(Dns {
        domain: SHERPA_DOMAIN_NAME.to_owned(),
        name_servers: vec![NameServer {
            ipv4_address,
            ipv6_address: Some(ipv6_address),
        }],
    })
}
