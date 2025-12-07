use std::net::Ipv4Addr;

use super::ZtpMethods;

#[derive(Clone, Debug)]
pub struct ZtpRecord {
    pub device_name: String,
    pub config_file: String,
    pub ipv4_address: Ipv4Addr,
    pub mac_address: String,
    pub ztp_method: ZtpMethods,
    pub ssh_port: u16,
}
