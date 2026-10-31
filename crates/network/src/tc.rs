use anyhow::{Context, Result};
use rtnetlink::packet_core::DefaultNla;
use rtnetlink::packet_core::{
    NLM_F_ACK, NLM_F_CREATE, NLM_F_REPLACE, NLM_F_REQUEST, NetlinkMessage, NetlinkPayload,
};
use rtnetlink::packet_route::RouteNetlinkMessage;
use rtnetlink::packet_route::tc::{TcAttribute, TcHandle, TcMessage};
use shared::data::LinkImpairment;
use shared::konst::{NETEM_QOPT_LEN, TCA_NETEM_CORRUPT, TCA_NETEM_REORDER, TCA_OPTIONS};
use tracing::instrument;

use crate::linux::setup_netlink;

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

/// Build netem's `TCA_OPTIONS` payload.
///
/// The kernel reads a raw `tc_netem_qopt` first, with no attribute header, then any netem
/// attributes after it. Reordering and corruption have no field in `tc_netem_qopt`, so they
/// go in `TCA_NETEM_REORDER` and `TCA_NETEM_CORRUPT`.
fn netem_options(impairment: &LinkImpairment) -> Vec<u8> {
    // latency and jitter are in PSCHED ticks, not microseconds. Modern kernels use a fixed
    // 15.625 MHz PSCHED clock: ticks = µs * 15625 / 1000.
    let latency = (impairment.delay_us as u64 * 15625 / 1000) as u32;
    let jitter = (impairment.jitter_us as u64 * 15625 / 1000) as u32;
    // With a gap of 1 and a reorder probability, each packet may be sent at once while the
    // rest wait for the delay, as `tc ... reorder` does.
    let gap: u32 = if impairment.reorder_percent > 0.0 {
        1
    } else {
        0
    };
    let mut options = Vec::with_capacity(NETEM_QOPT_LEN + 24);
    options.extend_from_slice(&latency.to_ne_bytes());
    options.extend_from_slice(&1000u32.to_ne_bytes()); // limit: default queue depth
    options.extend_from_slice(&percent_to_kernel(impairment.loss_percent).to_ne_bytes());
    options.extend_from_slice(&gap.to_ne_bytes());
    options.extend_from_slice(&0u32.to_ne_bytes()); // duplicate
    options.extend_from_slice(&jitter.to_ne_bytes());

    if impairment.reorder_percent > 0.0 {
        push_probability_attribute(&mut options, TCA_NETEM_REORDER, impairment.reorder_percent);
    }
    if impairment.corrupt_percent > 0.0 {
        push_probability_attribute(&mut options, TCA_NETEM_CORRUPT, impairment.corrupt_percent);
    }
    options
}

/// Append a netem attribute holding a probability and a correlation (always 0), the layout of
/// both `tc_netem_reorder` and `tc_netem_corrupt`. It is 12 bytes, so no padding is needed.
fn push_probability_attribute(options: &mut Vec<u8>, kind: u16, percent: f32) {
    options.extend_from_slice(&12u16.to_ne_bytes());
    options.extend_from_slice(&kind.to_ne_bytes());
    options.extend_from_slice(&percent_to_kernel(percent).to_ne_bytes());
    options.extend_from_slice(&0u32.to_ne_bytes());
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
    let mut msg = TcMessage::default();
    msg.header.index = iface_index;
    msg.header.handle = TcHandle { major: 1, minor: 0 };
    msg.header.parent = TcHandle::ROOT;
    msg.attributes.push(TcAttribute::Kind("netem".to_string()));
    msg.attributes.push(TcAttribute::Other(DefaultNla::new(
        TCA_OPTIONS,
        netem_options(impairment),
    )));

    let mut req = NetlinkMessage::from(RouteNetlinkMessage::NewQueueDiscipline(msg));
    req.header.flags = NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_REPLACE;

    tracing::info!(
        iface_index = iface_index,
        delay_us = impairment.delay_us,
        jitter_us = impairment.jitter_us,
        loss_pct = impairment.loss_percent,
        reorder_pct = impairment.reorder_percent,
        corrupt_pct = impairment.corrupt_percent,
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

/// Apply a link's impairment for each direction that has one. Traffic from A to B leaves the
/// host through B's end, so `a_to_b` goes on `ifindex_b`'s egress and `b_to_a` on `ifindex_a`'s.
#[instrument(skip(a_to_b, b_to_a), fields(ifindex_a, ifindex_b), level = "debug")]
pub async fn apply_link_netem(
    ifindex_a: u32,
    ifindex_b: u32,
    a_to_b: &LinkImpairment,
    b_to_a: &LinkImpairment,
) -> Result<()> {
    if !a_to_b.is_clear() {
        apply_netem(ifindex_b as i32, a_to_b)
            .await
            .context(format!("failed to apply netem on ifindex {ifindex_b}"))?;
    }
    if !b_to_a.is_clear() {
        apply_netem(ifindex_a as i32, b_to_a)
            .await
            .context(format!("failed to apply netem on ifindex {ifindex_a}"))?;
    }
    Ok(())
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

    /// Length, type and the two u32 fields of the netem attribute at `offset`.
    fn nla(options: &[u8], offset: usize) -> (u16, u16, u32, u32) {
        let len = u16::from_ne_bytes(options[offset..offset + 2].try_into().unwrap());
        let kind = u16::from_ne_bytes(options[offset + 2..offset + 4].try_into().unwrap());
        let probability = u32::from_ne_bytes(options[offset + 4..offset + 8].try_into().unwrap());
        let correlation = u32::from_ne_bytes(options[offset + 8..offset + 12].try_into().unwrap());
        (len, kind, probability, correlation)
    }

    #[test]
    fn test_netem_options_without_reorder_or_corrupt_is_only_qopt() {
        let options = netem_options(&LinkImpairment {
            delay_us: 20_000,
            loss_percent: 10.0,
            ..LinkImpairment::default()
        });
        assert_eq!(options.len(), NETEM_QOPT_LEN);
        assert_eq!(&options[0..4], &312_500u32.to_ne_bytes());
        assert_eq!(&options[8..12], &percent_to_kernel(10.0).to_ne_bytes());
    }

    #[test]
    fn test_netem_options_carry_reorder() {
        let options = netem_options(&LinkImpairment {
            delay_us: 10_000,
            reorder_percent: 25.0,
            ..LinkImpairment::default()
        });
        assert_eq!(&options[12..16], &1u32.to_ne_bytes(), "gap");
        assert_eq!(options.len(), NETEM_QOPT_LEN + 12);
        assert_eq!(
            nla(&options, NETEM_QOPT_LEN),
            (12, TCA_NETEM_REORDER, percent_to_kernel(25.0), 0)
        );
    }

    #[test]
    fn test_netem_options_carry_corrupt() {
        let options = netem_options(&LinkImpairment {
            corrupt_percent: 5.0,
            ..LinkImpairment::default()
        });
        assert_eq!(options.len(), NETEM_QOPT_LEN + 12);
        assert_eq!(
            nla(&options, NETEM_QOPT_LEN),
            (12, TCA_NETEM_CORRUPT, percent_to_kernel(5.0), 0)
        );
    }

    #[test]
    fn test_netem_options_carry_reorder_and_corrupt() {
        let options = netem_options(&LinkImpairment {
            delay_us: 10_000,
            reorder_percent: 25.0,
            corrupt_percent: 5.0,
            ..LinkImpairment::default()
        });
        assert_eq!(options.len(), NETEM_QOPT_LEN + 24);
        assert_eq!(nla(&options, NETEM_QOPT_LEN).1, TCA_NETEM_REORDER);
        assert_eq!(nla(&options, NETEM_QOPT_LEN + 12).1, TCA_NETEM_CORRUPT);
    }

    #[test]
    fn test_percent_to_kernel_small_value() {
        let result = percent_to_kernel(0.1);
        assert!(result > 0, "0.1% should produce non-zero kernel value");
    }
}
