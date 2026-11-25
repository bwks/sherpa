use std::net::{Ipv4Addr, Ipv6Addr};

use serde::{Serialize as SerializeTrait, Serializer};
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
}

#[derive(Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CiscoFxosFirewallMode {
    #[default]
    Routed,
    Transparent,
}

#[derive(Serialize, Default)]
pub struct CiscoFtdZtpTemplate {
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
    pub ipv6_mask: Option<PrefixLength>,
    #[serde(rename = "ManageLocally")]
    pub manage_locally: YesNo,
}

/// An IPv6 prefix length, which the `FTDv` day-0 configuration takes as a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrefixLength(pub u8);

impl SerializeTrait for PrefixLength {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

/// A boolean the `FTDv` day-0 configuration spells `Yes` or `No`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YesNo(pub bool);

impl SerializeTrait for YesNo {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(if self.0 { "Yes" } else { "No" })
    }
}
