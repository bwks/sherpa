use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

use anyhow::{Context, Result};
use ipnet::Ipv4Net;
use serde_derive::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct LabInfo {
    pub id: String,
    pub name: String,
    pub user: String,
    pub ipv4_network: Ipv4Net,
    pub ipv4_gateway: Ipv4Addr,
    pub ipv4_router: Ipv4Addr,
}
impl fmt::Display for LabInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let toml_string = toml::to_string_pretty(self).map_err(|_| fmt::Error)?;
        write!(f, "{}", toml_string)
    }
}
impl FromStr for LabInfo {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        toml::from_str(s).context("Failed to parse LabInfo from TOML")
    }
}
