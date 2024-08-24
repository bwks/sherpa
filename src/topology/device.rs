use serde_derive::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Device {
    pub name: String,
    pub num_interfaces: u8,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Interface {
    pub name: String,
}
