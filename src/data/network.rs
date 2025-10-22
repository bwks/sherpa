use anyhow::{anyhow, Result};

use ipnetwork::Ipv4Network;
use std::net::Ipv4Addr;

pub struct ManagementNetwork {
    pub network: Ipv4Network,
    pub network_ip: Ipv4Addr,
    pub gateway_ip: Ipv4Addr,
    pub subnet_mask: Ipv4Addr,
    pub prefix_length: u8,
}
impl ManagementNetwork {
    pub fn from_str(network: &str) -> Result<Self> {
        let net = network.parse::<Ipv4Network>()?;
        Ok(Self {
            network: net,
            network_ip: net.network(),
            gateway_ip: net.nth(1).unwrap(), // this should never fail
            subnet_mask: net.mask(),
            prefix_length: net.prefix(),
        })
    }

    pub fn get_ip(&self, nth: u32) -> Result<Ipv4Addr> {
        self.network
            .nth(nth)
            .ok_or_else(|| anyhow!("Failed to get IP:{nth} from network: {}", self.network))
    }
}
