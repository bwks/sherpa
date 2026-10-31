use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use shared::data::{LinkImpairment, NodeModel};
use shared::util::split_node_int;

/// Manifest Link
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Link {
    pub node_a: String,
    pub int_a: u8,
    pub node_b: String,
    pub int_b: u8,
}

/// Link impairment configuration from the manifest.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestImpairment {
    /// One-way delay in milliseconds.
    pub delay: Option<u32>,
    /// Delay jitter in milliseconds.
    pub jitter: Option<u32>,
    /// Packet loss percentage (0.0-100.0).
    pub loss_percent: Option<f32>,
    /// Packet reordering percentage (0.0-100.0).
    pub reorder_percent: Option<f32>,
    /// Bit-flip corruption percentage (0.0-100.0).
    pub corrupt_percent: Option<f32>,
}

impl From<&ManifestImpairment> for LinkImpairment {
    /// The manifest gives delay and jitter in milliseconds; netem takes microseconds.
    fn from(value: &ManifestImpairment) -> Self {
        Self {
            delay_us: value.delay.unwrap_or(0) * 1000,
            jitter_us: value.jitter.unwrap_or(0) * 1000,
            loss_percent: value.loss_percent.unwrap_or(0.0),
            reorder_percent: value.reorder_percent.unwrap_or(0.0),
            corrupt_percent: value.corrupt_percent.unwrap_or(0.0),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LinkExpanded {
    pub node_a: String,
    pub int_a: String,
    pub node_b: String,
    pub int_b: String,
    pub impairment: Option<ManifestImpairment>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LinkDetailed {
    pub node_a: String,
    pub node_a_idx: u16,
    pub node_a_model: NodeModel,
    pub int_a: String,
    pub int_a_idx: u8,
    pub node_b: String,
    pub node_b_idx: u16,
    pub node_b_model: NodeModel,
    pub int_b: String,
    pub int_b_idx: u8,
    pub link_idx: u16,
    pub impairment: Option<ManifestImpairment>,
}

/// Manifest Link
/// expected format:
/// "node_name::int_name" - Seperated by double collon `::`
/// {src = "<src_node_name>::<src_int_name>", dst = "<dst_node_name>::<dst_int_name>" }
/// {src = "dev01::eth1", dst = "dev02::eth0" }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Link2 {
    pub src: String,
    pub dst: String,
    /// Removed: every link is a P2p (eBPF) link. Kept only so validation can reject a manifest
    /// that still sets it (`validate::check_no_p2p_field`).
    pub p2p: Option<bool>,
    pub impairment: Option<ManifestImpairment>,
}

impl Link2 {
    pub fn expand(&self) -> Result<LinkExpanded> {
        let (node_a, int_a) = split_node_int(&self.src)?;
        let (node_b, int_b) = split_node_int(&self.dst)?;

        Ok(LinkExpanded {
            node_a,
            int_a,
            node_b,
            int_b,
            impairment: self.impairment.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_impairment_converts_milliseconds_to_microseconds() {
        let manifest = ManifestImpairment {
            delay: Some(20),
            jitter: Some(2),
            loss_percent: Some(1.5),
            reorder_percent: Some(10.0),
            corrupt_percent: Some(0.5),
        };
        assert_eq!(
            LinkImpairment::from(&manifest),
            LinkImpairment {
                delay_us: 20_000,
                jitter_us: 2_000,
                loss_percent: 1.5,
                reorder_percent: 10.0,
                corrupt_percent: 0.5,
            }
        );
    }

    #[test]
    fn test_empty_manifest_impairment_is_clear() {
        assert!(LinkImpairment::from(&ManifestImpairment::default()).is_clear());
    }
}
