use anyhow::{Context, Result};
use rtnetlink::packet_core::DefaultNla;
use rtnetlink::packet_core::{
    NLM_F_ACK, NLM_F_CREATE, NLM_F_REPLACE, NLM_F_REQUEST, NetlinkMessage, NetlinkPayload,
};
use rtnetlink::packet_route::RouteNetlinkMessage;
use rtnetlink::packet_route::tc::{TcAttribute, TcHandle, TcMessage};
use tracing::instrument;

use crate::linux::setup_netlink;

/// Link impairment parameters for TC netem.
#[derive(Debug, Clone, Default)]
pub struct LinkImpairment {
    /// One-way delay in microseconds.
    pub delay_us: u32,
    /// Delay jitter in microseconds.
    pub jitter_us: u32,
    /// Packet loss in percent (0.0-100.0).
    pub loss_percent: f32,
    /// Packet reordering probability as percent (0.0-100.0).
    pub reorder_percent: f32,
    /// Bit-flip corruption probability as percent (0.0-100.0).
    pub corrupt_percent: f32,
}

/// Convert a percentage (0.0-100.0) to the kernel's u32 representation.
/// Kernel uses 0 = 0%, u32::MAX ~ 100%.
fn percent_to_kernel(pct: f32) -> u32 {
    if pct <= 0.0 {
        return 0;
    }
    if pct >= 100.0 {
        return u32::MAX;
    }
    ((pct as f64 / 100.0) * u32::MAX as f64) as u32
}

/// Send a raw TC netlink message and check for errors in the response.
async fn send_tc_message(msg: NetlinkMessage<RouteNetlinkMessage>) -> Result<()> {
    let mut handle = setup_netlink().await?;
    let mut response = handle
        .request(msg)
        .context("failed to send TC netlink request")?;

    while let Some(message) = futures::StreamExt::next(&mut response).await {
        if let NetlinkPayload::Error(err) = message.payload
            && err.code.is_some()
        {
            return Err(anyhow::anyhow!("TC netlink error: {err}"));
        }
    }

    Ok(())
}

/// Apply a netem qdisc to an interface for link impairment simulation.
///
/// This sets the root qdisc to netem with the specified impairment parameters.
/// Can coexist with the clsact qdisc that aya uses for TC BPF programs,
/// because clsact handles ingress/egress classification while netem is the
/// root egress scheduler.
#[instrument(fields(iface_index, delay_us = impairment.delay_us, jitter_us = impairment.jitter_us), level = "debug")]
pub async fn apply_netem(iface_index: i32, impairment: &LinkImpairment) -> Result<()> {
    // Serialize tc_netem_qopt fields in kernel layout order (6 x u32, native endian).
    // latency and jitter must be in PSCHED ticks (not microseconds).
    // Modern Linux kernels use a fixed 15.625 MHz PSCHED clock: ticks = µs * 15625 / 1000.
    let latency = (impairment.delay_us as u64 * 15625 / 1000) as u32;
    let jitter = (impairment.jitter_us as u64 * 15625 / 1000) as u32;
    let gap: u32 = if impairment.reorder_percent > 0.0 {
        1
    } else {
        0
    };
    let mut qopt_bytes = Vec::with_capacity(24);
    qopt_bytes.extend_from_slice(&latency.to_ne_bytes());
    qopt_bytes.extend_from_slice(&1000u32.to_ne_bytes()); // limit: default queue depth
    qopt_bytes.extend_from_slice(&percent_to_kernel(impairment.loss_percent).to_ne_bytes());
    qopt_bytes.extend_from_slice(&gap.to_ne_bytes());
    qopt_bytes.extend_from_slice(&0u32.to_ne_bytes()); // duplicate
    qopt_bytes.extend_from_slice(&jitter.to_ne_bytes());

    let mut msg = TcMessage::default();
    msg.header.index = iface_index;
    msg.header.handle = TcHandle { major: 1, minor: 0 };
    msg.header.parent = TcHandle::ROOT;
    msg.attributes.push(TcAttribute::Kind("netem".to_string()));
    // TCA_OPTIONS (type 2) must carry the raw tc_netem_qopt bytes directly —
    // the kernel reads them without any nested NLA framing.
    msg.attributes
        .push(TcAttribute::Other(DefaultNla::new(2, qopt_bytes)));

    let mut req = NetlinkMessage::from(RouteNetlinkMessage::NewQueueDiscipline(msg));
    req.header.flags = NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_REPLACE;

    tracing::info!(
        iface_index = iface_index,
        delay_us = impairment.delay_us,
        jitter_us = impairment.jitter_us,
        loss_pct = impairment.loss_percent,
        "applying netem qdisc"
    );

    send_tc_message(req).await
}

/// Remove the netem qdisc from an interface.
#[instrument(fields(iface_index), level = "debug")]
pub async fn remove_netem(iface_index: i32) -> Result<()> {
    let mut msg = TcMessage::default();
    msg.header.index = iface_index;
    msg.header.handle = TcHandle { major: 1, minor: 0 };
    msg.header.parent = TcHandle::ROOT;

    let mut req = NetlinkMessage::from(RouteNetlinkMessage::DelQueueDiscipline(msg));
    req.header.flags = NLM_F_REQUEST | NLM_F_ACK;

    tracing::info!(iface_index = iface_index, "removing netem qdisc");

    send_tc_message(req).await
}

/// Update netem parameters on an interface that already has netem applied.
///
/// This replaces the existing netem qdisc with updated parameters.
#[instrument(skip(impairment), fields(iface_index), level = "debug")]
pub async fn update_netem(iface_index: i32, impairment: &LinkImpairment) -> Result<()> {
    // Remove then re-add — simplest approach that avoids change vs replace semantics
    let _ = remove_netem(iface_index).await;
    apply_netem(iface_index, impairment).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percent_to_kernel_zero() {
        assert_eq!(percent_to_kernel(0.0), 0);
    }

    #[test]
    fn test_percent_to_kernel_hundred() {
        assert_eq!(percent_to_kernel(100.0), u32::MAX);
    }

    #[test]
    fn test_percent_to_kernel_fifty() {
        let result = percent_to_kernel(50.0);
        // 50% should be roughly half of u32::MAX
        let expected = (u32::MAX as f64 / 2.0) as u32;
        assert!(
            result.abs_diff(expected) < 2,
            "50% should be ~{}, got {}",
            expected,
            result
        );
    }

    #[test]
    fn test_percent_to_kernel_negative_clamps() {
        assert_eq!(percent_to_kernel(-5.0), 0);
    }

    #[test]
    fn test_percent_to_kernel_over_hundred_clamps() {
        assert_eq!(percent_to_kernel(150.0), u32::MAX);
    }

    #[test]
    fn test_percent_to_kernel_small_value() {
        let result = percent_to_kernel(0.1);
        assert!(result > 0, "0.1% should produce non-zero kernel value");
    }
}
