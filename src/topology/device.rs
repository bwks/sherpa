use serde_derive::{Deserialize, Serialize};

use crate::data::DeviceModels;

#[derive(Debug, Deserialize, Default, Serialize)]
pub struct Device {
    pub name: String,
    pub device_model: DeviceModels,
}
