use serde_derive::{Deserialize, Serialize};

use crate::model::DeviceModels;

#[derive(Debug, Deserialize, Serialize)]
pub struct Device {
    pub name: String,
    pub device_model: DeviceModels,
}
