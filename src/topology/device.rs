use std::borrow::Cow;

use serde_derive::{Deserialize, Serialize};

use crate::model::{DeviceModel, DeviceModels};

#[derive(Debug, Deserialize, Serialize)]
pub struct Device {
    pub name: String,
    pub device_type: DeviceModels,
    pub device_model: Option<DeviceModel>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Interface {
    pub name: String,
    pub num: u8,
}
