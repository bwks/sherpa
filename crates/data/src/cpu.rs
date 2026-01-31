use std::fmt;

use serde_derive::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq, Eq, EnumIter)]
pub enum CpuModels {
    #[default]
    #[serde(rename(serialize = "host-model", deserialize = "host-model"))]
    HostModel,
    #[serde(rename(serialize = "IvyBridge", deserialize = "IvyBridge"))]
    IvyBridge,
    #[serde(rename(serialize = "SandyBridge", deserialize = "SandyBridge"))]
    SandyBridge,
}
impl fmt::Display for CpuModels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CpuModels::HostModel => write!(f, "host-model"),
            CpuModels::IvyBridge => write!(f, "IvyBridge"),
            CpuModels::SandyBridge => write!(f, "SandyBridge"),
        }
    }
}
impl CpuModels {
    pub fn to_vec() -> Vec<CpuModels> {
        CpuModels::iter().collect()
    }
}
