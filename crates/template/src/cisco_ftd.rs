use std::net::{Ipv4Addr, Ipv6Addr};

use serde::{Serialize as SerializeTrait, Serializer};
use serde_derive::Serialize;

use shared::data::{NetworkV4, NetworkV6};

/*
"EULA": "accept",
"Hostname": "inserthostname-here",
"AdminPassword": "Cisc01@3",
"FirewallMode": "routed",
"DNS1": "",
"DNS2": "",
"DNS3": "",
"IPv4Mode": "manual",
"IPv4Addr": "",
"IPv4Mask": "",
"IPv4Gw": "",
"IPv6Mode": "disabled",
"IPv6Addr": "",
"IPv6Mask": "",
"IPv6Gw": "",
"FmcIp": "",
"FmcRegKey": "",
"FmcNatId": "",
"ManageLocally":"Yes"
*/

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum CiscoFxosIpMode {
    Manual,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CiscoFxosFirewallMode {
    Routed,
    Transparent,
}

pub struct CiscoFtdZtpTemplate {
    pub eula: String,
    pub hostname: String,
    pub admin_password: String,
    pub firewall_mode: CiscoFxosFirewallMode,
    pub dns: Ipv4Addr,
    pub manage_locally: bool,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Ipv4Addr,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
}

#[derive(Serialize)]
struct Day0Config<'a> {
    #[serde(rename = "EULA")]
    eula: &'a str,
    #[serde(rename = "Hostname")]
    hostname: &'a str,
    #[serde(rename = "AdminPassword")]
    admin_password: &'a str,
    #[serde(rename = "FirewallMode")]
    firewall_mode: &'a CiscoFxosFirewallMode,
    #[serde(rename = "DNS1")]
    dns1: Ipv4Addr,
    #[serde(rename = "IPv4Mode")]
    ipv4_mode: CiscoFxosIpMode,
    #[serde(rename = "IPv4Addr")]
    ipv4_addr: Ipv4Addr,
    #[serde(rename = "IPv4Gw")]
    ipv4_gw: Ipv4Addr,
    #[serde(rename = "IPv4Mask")]
    ipv4_mask: Ipv4Addr,
    #[serde(flatten)]
    ipv6: Option<Day0Ipv6>,
    #[serde(rename = "ManageLocally")]
    manage_locally: &'static str,
}

#[derive(Serialize)]
struct Day0Ipv6 {
    #[serde(rename = "IPv6Mode")]
    mode: CiscoFxosIpMode,
    #[serde(rename = "IPv6Addr")]
    addr: Ipv6Addr,
    #[serde(rename = "IPv6Gw")]
    gw: Ipv6Addr,
    #[serde(rename = "IPv6Mask")]
    mask: String,
}

impl SerializeTrait for CiscoFtdZtpTemplate {
    /// Serializes the template as `FTDv`'s day-0 JSON: manual IPv4 and, with both an IPv6 address
    /// and an IPv6 network, manual IPv6, each with the network's first address as the gateway,
    /// and `ManageLocally` as `Yes` or `No`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let ipv6 = match (self.mgmt_ipv6_address, &self.mgmt_ipv6) {
            (Some(address), Some(network)) => Some(Day0Ipv6 {
                mode: CiscoFxosIpMode::Manual,
                addr: address,
                gw: network.first,
                mask: network.prefix_length.to_string(),
            }),
            _ => None,
        };
        Day0Config {
            eula: &self.eula,
            hostname: &self.hostname,
            admin_password: &self.admin_password,
            firewall_mode: &self.firewall_mode,
            dns1: self.dns,
            ipv4_mode: CiscoFxosIpMode::Manual,
            ipv4_addr: self.mgmt_ipv4_address,
            ipv4_gw: self.mgmt_ipv4.first,
            ipv4_mask: self.mgmt_ipv4.subnet_mask,
            ipv6,
            manage_locally: if self.manage_locally { "Yes" } else { "No" },
        }
        .serialize(serializer)
    }
}
