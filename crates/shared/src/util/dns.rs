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
