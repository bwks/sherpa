use std::net::{Ipv4Addr, Ipv6Addr};

use serde::Serializer;
use serde_derive::Serialize;

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
pub enum CiscoFxosIpMode {
    Manual,
    Dhcp,
    Disabled,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CiscoFxosFirewallMode {
    #[default]
    Routed,
    Transparent,
}

#[derive(Serialize, Default)]
pub struct CiscoFtdvZtpTemplate {
    #[serde(rename = "EULA")]
    pub eula: String,
    #[serde(rename = "Hostname")]
    pub hostname: String,
    #[serde(rename = "AdminPassword")]
    pub admin_password: String,
    #[serde(rename = "FirewallMode")]
    pub firewall_mode: CiscoFxosFirewallMode,
    #[serde(rename = "DNS1", skip_serializing_if = "Option::is_none")]
    pub dns1: Option<Ipv4Addr>,
    #[serde(rename = "DNS2", skip_serializing_if = "Option::is_none")]
    pub dns2: Option<Ipv4Addr>,
    #[serde(rename = "DNS3", skip_serializing_if = "Option::is_none")]
    pub dns3: Option<Ipv4Addr>,
    #[serde(rename = "IPv4Mode", skip_serializing_if = "Option::is_none")]
    pub ipv4_mode: Option<CiscoFxosIpMode>,
    #[serde(rename = "IPv4Addr", skip_serializing_if = "Option::is_none")]
    pub ipv4_addr: Option<Ipv4Addr>,
    #[serde(rename = "IPv4Gw", skip_serializing_if = "Option::is_none")]
    pub ipv4_gw: Option<Ipv4Addr>,
    #[serde(rename = "IPv4Mask", skip_serializing_if = "Option::is_none")]
    pub ipv4_mask: Option<Ipv4Addr>,
    #[serde(rename = "IPv6Mode", skip_serializing_if = "Option::is_none")]
    pub ipv6_mode: Option<CiscoFxosIpMode>,
    #[serde(rename = "IPv6Addr", skip_serializing_if = "Option::is_none")]
    pub ipv6_addr: Option<Ipv6Addr>,
    #[serde(rename = "IPv6Gw", skip_serializing_if = "Option::is_none")]
    pub ipv6_gw: Option<Ipv6Addr>,
    #[serde(rename = "IPv6Mask", skip_serializing_if = "Option::is_none")]
    pub ipv6_mask: Option<u8>,
    #[serde(rename = "ManageLocally", serialize_with = "serialize_yes_no")]
    pub manage_locally: bool,
}

fn serialize_yes_no<S>(value: &bool, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(if *value { "Yes" } else { "No" })
}
