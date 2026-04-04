use std::fmt;

use schemars::JsonSchema;
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

#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq, Eq, EnumIter, JsonSchema)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_feature_policy_display_require() {
        assert_eq!(CpuFeaturePolicy::Require.to_string(), "require");
    }

    #[test]
    fn test_cpu_feature_policy_display_disable() {
        assert_eq!(CpuFeaturePolicy::Disable.to_string(), "disable");
    }

    #[test]
    fn test_cpu_models_display_host_model() {
        assert_eq!(CpuModels::HostModel.to_string(), "host-model");
    }

    #[test]
    fn test_cpu_models_display_ivy_bridge() {
        assert_eq!(CpuModels::IvyBridge.to_string(), "IvyBridge");
    }

    #[test]
    fn test_cpu_models_display_sandy_bridge() {
        assert_eq!(CpuModels::SandyBridge.to_string(), "SandyBridge");
    }

    #[test]
    fn test_cpu_models_default_is_host_model() {
        assert_eq!(CpuModels::default(), CpuModels::HostModel);
    }

    #[test]
    fn test_cpu_models_to_vec_contains_all_variants() {
        let models = CpuModels::to_vec();
        assert_eq!(models.len(), 3);
        assert!(models.iter().any(|m| matches!(m, CpuModels::HostModel)));
        assert!(models.iter().any(|m| matches!(m, CpuModels::IvyBridge)));
        assert!(models.iter().any(|m| matches!(m, CpuModels::SandyBridge)));
    }

    #[test]
    fn test_cpu_models_serde_round_trip() {
        for model in CpuModels::to_vec() {
            let json = serde_json::to_string(&model).unwrap();
            let deserialized: CpuModels = serde_json::from_str(&json).unwrap();
            assert_eq!(model, deserialized);
        }
    }
}
