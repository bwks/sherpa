use std::net::{Ipv4Addr, Ipv6Addr};

use super::ZtpMethod;

#[derive(Clone, Debug)]
pub struct ZtpRecord {
    pub node_name: String,
    pub config_file: String,
    pub ipv4_address: Ipv4Addr,
    pub ipv6_address: Option<Ipv6Addr>,
    pub mac_address: String,
    pub ztp_method: ZtpMethod,
    pub ssh_port: u16,
}
