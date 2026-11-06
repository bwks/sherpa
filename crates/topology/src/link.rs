use anyhow::{Context, Error, Result};
use serde_derive::{Deserialize, Serialize};

use shared::data::{LinkImpairment, NodeModel};
use shared::util::split_node_int;

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

impl TryFrom<&ManifestImpairment> for LinkImpairment {
    type Error = Error;

    /// The manifest gives delay and jitter in milliseconds; netem takes microseconds.
    fn try_from(value: &ManifestImpairment) -> Result<Self> {
        let delay = value.delay.unwrap_or(0);
        let jitter = value.jitter.unwrap_or(0);
        Ok(Self {
            delay_us: delay
                .checked_mul(1000)
                .with_context(|| format!("Impairment delay of {delay} ms is too long"))?,
            jitter_us: jitter
                .checked_mul(1000)
                .with_context(|| format!("Impairment jitter of {jitter} ms is too long"))?,
            loss_percent: value.loss_percent.unwrap_or(0.0),
            reorder_percent: value.reorder_percent.unwrap_or(0.0),
            corrupt_percent: value.corrupt_percent.unwrap_or(0.0),
        })
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

/// A link in the manifest, between two `node_name::int_name` ends:
/// `{ src = "dev01::eth1", dst = "dev02::eth0" }`.
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
    /// Split both ends into node and interface names.
    ///
    /// # Errors
    ///
    /// Fails when an end is not `node_name::int_name`.
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
            LinkImpairment::try_from(&manifest).unwrap(),
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
        assert!(
            LinkImpairment::try_from(&ManifestImpairment::default())
                .unwrap()
                .is_clear()
        );
    }

    #[test]
    fn test_manifest_impairment_too_long_for_microseconds_is_refused() {
        let manifest = ManifestImpairment {
            delay: Some(u32::MAX),
            ..ManifestImpairment::default()
        };
        assert!(LinkImpairment::try_from(&manifest).is_err());
    }
}
