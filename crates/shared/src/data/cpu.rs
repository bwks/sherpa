use std::fmt;

use serde_derive::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

/// A CPU feature with a policy for libvirt domain XML.
#[derive(Clone, Debug)]
pub struct CpuFeature {
    pub name: String,
    pub policy: CpuFeaturePolicy,
}

#[derive(Clone, Debug)]
pub enum CpuFeaturePolicy {
    Require,
    Disable,
}

impl fmt::Display for CpuFeaturePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CpuFeaturePolicy::Require => write!(f, "require"),
            CpuFeaturePolicy::Disable => write!(f, "disable"),
        }
    }
}

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
impl_surreal_value_for_enum!(CpuModels);
