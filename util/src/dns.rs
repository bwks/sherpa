use anyhow::Result;
use ipnet::Ipv4Net;

use super::ip::get_ipv4_addr;
use data::Dns;
use data::NameServer;
use konst::{BOOT_SERVER_NAME, SHERPA_DOMAIN_NAME};

pub fn default_dns(mgmt_net: &Ipv4Net) -> Result<Dns> {
    let ipv4_address = get_ipv4_addr(&mgmt_net, 1)?;

    Ok(Dns {
        domain: SHERPA_DOMAIN_NAME.to_owned(),
        name_servers: vec![NameServer {
            name: BOOT_SERVER_NAME.to_owned(),
            ipv4_address,
        }],
    })
}
