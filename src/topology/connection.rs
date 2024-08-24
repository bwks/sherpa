use serde_derive::{Deserialize, Serialize};

use super::{Device, Interface};

#[derive(Debug, Deserialize, Serialize)]
pub struct Connection {
    device_a: String,
    interface_a: u8,
    device_b: String,
    interface_b: u8,
}
