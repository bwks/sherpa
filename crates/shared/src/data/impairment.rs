use clap::ValueEnum;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which direction of a link an impairment change applies to. `src` and `dst` are the link's
/// ends as the manifest names them.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum ImpairmentDirection {
    /// Both directions.
    #[default]
    Both,
    /// Traffic sent by `src` towards `dst`.
    SrcToDst,
    /// Traffic sent by `dst` towards `src`.
    DstToSrc,
}

/// Link impairment parameters for one direction of a link, applied with TC netem.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct LinkImpairment {
    /// One-way delay in microseconds.
    #[serde(default)]
    pub delay_us: u32,
    /// Delay jitter in microseconds.
    #[serde(default)]
    pub jitter_us: u32,
    /// Packet loss in percent (0.0-100.0).
    #[serde(default)]
    pub loss_percent: f32,
    /// Packet reordering probability as percent (0.0-100.0).
    #[serde(default)]
    pub reorder_percent: f32,
    /// Bit-flip corruption probability as percent (0.0-100.0).
    #[serde(default)]
    pub corrupt_percent: f32,
}

impl LinkImpairment {
    /// True when nothing is impaired, so no netem qdisc is needed.
    pub fn is_clear(&self) -> bool {
        *self == Self::default()
    }
}

/// Request type for updating link impairment on a running lab.
/// Delay and jitter values are in milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateImpairmentRequest {
    pub lab_id: String,
    pub link_index: u16,
    /// The direction to change. The other direction keeps its impairment.
    #[serde(default)]
    pub direction: ImpairmentDirection,
    #[serde(default)]
    pub delay: u32,
    #[serde(default)]
    pub jitter: u32,
    #[serde(default)]
    pub loss_percent: f32,
    #[serde(default)]
    pub reorder_percent: f32,
    #[serde(default)]
    pub corrupt_percent: f32,
}

/// Response type for updating link impairment
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateImpairmentResponse {
    pub success: bool,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_impairment_is_clear() {
        assert!(LinkImpairment::default().is_clear());
    }

    #[test]
    fn test_any_parameter_makes_impairment_not_clear() {
        let reorder_only = LinkImpairment {
            reorder_percent: 5.0,
            ..LinkImpairment::default()
        };
        let corrupt_only = LinkImpairment {
            corrupt_percent: 0.1,
            ..LinkImpairment::default()
        };
        let jitter_only = LinkImpairment {
            jitter_us: 1000,
            ..LinkImpairment::default()
        };
        assert!(!reorder_only.is_clear());
        assert!(!corrupt_only.is_clear());
        assert!(!jitter_only.is_clear());
    }

    #[test]
    fn test_request_without_direction_changes_both() {
        let request: UpdateImpairmentRequest =
            serde_json::from_str(r#"{"lab_id":"a1b2c3d4","link_index":0,"delay":20}"#).unwrap();
        assert_eq!(request.direction, ImpairmentDirection::Both);
    }

    #[test]
    fn test_request_direction_is_snake_case() {
        let request: UpdateImpairmentRequest = serde_json::from_str(
            r#"{"lab_id":"a1b2c3d4","link_index":0,"direction":"dst_to_src","delay":20}"#,
        )
        .unwrap();
        assert_eq!(request.direction, ImpairmentDirection::DstToSrc);
    }
}
