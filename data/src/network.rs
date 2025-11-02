use std::net::Ipv4Addr;

use anyhow::{Result, anyhow};
use ipnetwork::Ipv4Network;

use crate::konst::{SHERPA_MANAGEMENT_NETWORK_IPV4, SHERPA_MANAGEMENT_VM_IPV4_INDEX};

pub struct NetworkV4 {
    pub prefix: Ipv4Network,
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
impl SherpaNetwork {
    pub fn new(prefix_v4: Option<&str>, _prefix_v6: Option<&str>) -> Result<Self> {
        let v4 = prefix_v4
            .unwrap_or(SHERPA_MANAGEMENT_NETWORK_IPV4)
            .parse::<Ipv4Network>()?;
        let first = v4
            .nth(1)
            .ok_or_else(|| anyhow!("Error parsing first IPv4"))?;
        let last = v4.broadcast();
        let boot_server = v4
            .nth(SHERPA_MANAGEMENT_VM_IPV4_INDEX)
            .ok_or_else(|| anyhow!("Error parsing boot server IPv4"))?;
        let subnet_mask = v4.mask();
        let network = v4.network();
        let v4_prefix = NetworkV4 {
            prefix: v4,
            first,
            last,
            boot_server,
            network,
            subnet_mask,
            prefix_length: v4.prefix(),
        };
        Ok(Self { v4: v4_prefix })
    }
}
