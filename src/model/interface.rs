use serde_derive::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Interface {
    pub name: String,
    pub num: u8,
    pub mac_address: String,
}
