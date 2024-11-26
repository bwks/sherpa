use serde_derive::{Deserialize, Serialize};

/// Manifest Connection
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Connection {
    pub device_a: String,
    pub interface_a: u8,
    pub device_b: String,
    pub interface_b: u8,
}
