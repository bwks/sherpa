use serde_derive::{Deserialize, Serialize};

use crate::topology::ConnectionMap;

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionTypes {
    #[default]
    Disabled, // Disable interface
    Management, // Connects to management bridge
    Peer,       // Peered with another device
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Interface {
    pub name: String,
    pub num: u8,
    pub mac_address: String,
    pub connection_type: ConnectionTypes,
    pub connection_map: Option<ConnectionMap>,
}
