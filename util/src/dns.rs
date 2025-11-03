use anyhow::Result;

use super::ip::{get_ipv4_addr, get_ipv4_network};
use data::Dns;
use data::NameServer;
use konst::{BOOT_SERVER_NAME, SHERPA_DOMAIN_NAME, SHERPA_MANAGEMENT_NETWORK_IPV4};

pub fn default_dns() -> Result<Dns> {
    let mgmt_net = get_ipv4_network(SHERPA_MANAGEMENT_NETWORK_IPV4)?;
    let ipv4_address = get_ipv4_addr(mgmt_net, 1)?;

    Ok(Dns {
        domain: SHERPA_DOMAIN_NAME.to_owned(),
        name_servers: vec![NameServer {
            name: BOOT_SERVER_NAME.to_owned(),
            ipv4_address,
        }],
    })
}
