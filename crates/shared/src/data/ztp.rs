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
    /// SSH port, or `None` for nodes without SSH (left out of the lab SSH config).
    pub ssh_port: Option<u16>,
}
