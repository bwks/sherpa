use serde_derive::{Deserialize, Serialize};

use crate::data::InterfaceConnection;

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum InterfaceTypes {
    #[default]
    Unknown,
    Mgmt,
    Eth,
    Swp,
    Gig,
    Ten,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionTypes {
    #[default]
    Disabled, // Disable interface
    Management, // Connects to management bridge
    Peer,       // Peered with another device
    Reserved,   // Reserved interfaces used by the virtual platform
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Interface {
    pub name: String,
    pub num: u8,
    pub mac_address: String,
    pub mtu: u16,
    pub connection_type: ConnectionTypes,
    pub interface_connection: Option<InterfaceConnection>,
}
