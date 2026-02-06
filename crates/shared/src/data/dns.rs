use std::net::Ipv4Addr;

#[derive(Clone)]
pub struct NameServer {
    #[allow(dead_code)]
    pub name: String,
    pub ipv4_address: Ipv4Addr,
}
// impl NameServer {
//     pub fn default() -> Result<Self> {
//         let mgmt_net = get_ipv4_network(SHERPA_MANAGEMENT_NETWORK_IPV4)?;

//         let ipv4_address = get_ipv4_addr(mgmt_net, 1)?;

//         Ok(Self {
//             name: BOOT_SERVER_NAME.to_owned(),
//             ipv4_address,
//         })
//     }
// }

#[derive(Clone)]
pub struct Dns {
    pub domain: String,
    pub name_servers: Vec<NameServer>,
}
// impl Dns {
//     pub fn default() -> Result<Self> {
//         Ok(Self {
//             domain: SHERPA_DOMAIN_NAME.to_owned(),
//             name_servers: vec![NameServer::default()?],
//         })
//     }
// }
