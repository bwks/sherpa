use ipnet::Ipv4Net;
use std::net::Ipv4Addr;

pub struct NetworkV4 {
    pub prefix: Ipv4Net,
    pub first: Ipv4Addr,
    pub last: Ipv4Addr,
    pub boot_server: Ipv4Addr,
    pub network: Ipv4Addr,
    pub subnet_mask: Ipv4Addr,
    pub prefix_length: u8,
}
// pub struct NetworkV6;

pub struct SherpaNetwork {
    pub v4: NetworkV4,
    // v6: SherpaManagementV6,
}
