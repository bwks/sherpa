use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Clone)]
pub struct NameServer {
    pub ipv4_address: Ipv4Addr,
    pub ipv6_address: Option<Ipv6Addr>,
}

#[derive(Clone)]
pub struct Dns {
    pub domain: String,
    pub name_servers: Vec<NameServer>,
}
