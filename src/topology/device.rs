use serde_derive::{Deserialize, Serialize};

use crate::data::DeviceModels;

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct Device {
    pub name: String,
    pub model: DeviceModels,
    pub cpu_count: Option<u8>,
    pub memory: Option<u16>,
}
