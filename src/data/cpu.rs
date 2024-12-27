use std::fmt;

use serde_derive::{Deserialize, Serialize};

#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CpuModels {
    #[default]
    HostModel,
    IvyBridge,
}
impl fmt::Display for CpuModels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CpuModels::HostModel => write!(f, "host-model"),
            CpuModels::IvyBridge => write!(f, "IvyBridge"),
        }
    }
}
