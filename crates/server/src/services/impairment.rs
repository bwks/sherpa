use anyhow::{Context, Result, anyhow};
use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use shared::data::{
    BridgeKind, ImpairmentDirection, LinkImpairment, UpdateImpairmentRequest,
    UpdateImpairmentResponse,
};
use shared::validators::validate_link_impairment;

/// Update link impairment on a running P2p link
///
/// This function:
/// 1. Finds the link in the database by `lab_id` + `link_index`
/// 2. Validates it is a P2p link (impairment only works on P2p links)
/// 3. Applies TC netem impairment to the host end of each direction the request names,
///    leaving the other direction as it was. All-zero values remove the impairment.
/// 4. Updates the database record with the new impairment values
///
/// The caller must own the lab or be an admin. `request.username` is ignored.
#[instrument(skip(ctx, state), fields(username = %ctx.username, lab_id = %request.lab_id, link_index = %request.link_index))]
pub async fn update_impairment(
    ctx: &AuthContext,
    request: UpdateImpairmentRequest,
    state: &AppState,
) -> Result<UpdateImpairmentResponse> {
    let lab_id = &request.lab_id;
    let impairment = requested_impairment(&request)?;

    // Get all links for this lab and find the one with the matching index
    let lab = require_lab_access(ctx, &state.db, lab_id).await?;

    let lab_record_id = lab
        .id
        .ok_or_else(|| anyhow!("Lab '{lab_id}' missing record ID"))?;

    let links = db::list_links_by_lab(&state.db, lab_record_id).await?;

    let mut db_link = links
        .into_iter()
        .find(|l| l.index == request.link_index)
        .ok_or_else(|| {
            anyhow!(
                "Link with index {} not found in lab '{}'",
                request.link_index,
                lab_id
            )
        })?;

    // Only P2p links support impairment
    if db_link.kind != BridgeKind::P2p {
        return Err(anyhow!(
            "Link impairment is only supported on P2p links (link index {} is {:?})",
            request.link_index,
            db_link.kind
        ));
    }

    // Traffic from A to B leaves the host through B's end, so its netem goes on tap_b.
    let (to_b, to_a) = match request.direction {
        ImpairmentDirection::Both => (true, true),
        ImpairmentDirection::SrcToDst => (true, false),
        ImpairmentDirection::DstToSrc => (false, true),
    };
    if to_b {
        set_netem(&db_link.tap_b, &impairment).await?;
    }
    if to_a && let Err(error) = set_netem(&db_link.tap_a, &impairment).await {
        // Put the other direction back, so the kernel matches the database again.
        if to_b && let Err(restore) = set_netem(&db_link.tap_b, &db_link.impairment_a_to_b).await {
            tracing::error!(
                lab_id = %lab_id,
                link_index = request.link_index,
                error = %restore,
                "Failed to restore the src to dst impairment; it no longer matches the database"
            );
        }
        return Err(error);
    }
    if to_b {
        db_link.impairment_a_to_b = impairment;
    }
    if to_a {
        db_link.impairment_b_to_a = impairment;
    }
    tracing::info!(
        lab_id = %lab_id,
        link_index = request.link_index,
        direction = ?request.direction,
        delay_ms = request.delay,
        jitter_ms = request.jitter,
        loss_percent = request.loss_percent,
        reorder_percent = request.reorder_percent,
        corrupt_percent = request.corrupt_percent,
        "Updated link impairment"
    );

    let link_record = db_link.id.as_ref().context("Link record has no id")?;
    db::update_link_impairments(
        &state.db,
        link_record,
        &db_link.impairment_a_to_b,
        &db_link.impairment_b_to_a,
    )
    .await
    .context("Failed to update link impairment in database")?;

    let direction = match request.direction {
        ImpairmentDirection::Both => "both directions",
        ImpairmentDirection::SrcToDst => "src to dst",
        ImpairmentDirection::DstToSrc => "dst to src",
    };
    let message = if impairment.is_clear() {
        format!(
            "Removed impairment from link {} in lab '{}' ({direction})",
            request.link_index, lab_id
        )
    } else {
        format!(
            "Applied impairment to link {} in lab '{}' ({direction}): delay={}ms jitter={}ms loss={}% reorder={}% corrupt={}%",
            request.link_index,
            lab_id,
            request.delay,
            request.jitter,
            request.loss_percent,
            request.reorder_percent,
            request.corrupt_percent,
        )
    };

    Ok(UpdateImpairmentResponse {
        success: true,
        message,
    })
}

/// The impairment a request asks for, with delays in microseconds as netem takes them.
fn requested_impairment(request: &UpdateImpairmentRequest) -> Result<LinkImpairment, ServiceError> {
    validate_link_impairment(
        request.delay,
        request.jitter,
        request.loss_percent,
        request.reorder_percent,
        request.corrupt_percent,
    )
    .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    Ok(LinkImpairment {
        delay_us: millis_to_micros(request.delay, "delay")?,
        jitter_us: millis_to_micros(request.jitter, "jitter")?,
        loss_percent: request.loss_percent,
        reorder_percent: request.reorder_percent,
        corrupt_percent: request.corrupt_percent,
    })
}

/// `ms` milliseconds of the impairment `field` in microseconds.
fn millis_to_micros(ms: u32, field: &str) -> Result<u32, ServiceError> {
    ms.checked_mul(1000)
        .ok_or_else(|| ServiceError::Invalid(format!("Impairment {field} of {ms} ms is too long")))
}

/// Replace the netem impairment on a host end, or remove it when `impairment` is clear.
#[instrument(skip(impairment), fields(%iface_name), level = "debug")]
async fn set_netem(iface_name: &str, impairment: &LinkImpairment) -> Result<()> {
    let ifindex = network::tap::get_ifindex(iface_name)
        .await
        .context(format!("Failed to get ifindex for {iface_name}"))?;
    if impairment.is_clear() {
        match network::tc::remove_netem(ifindex).await {
            Ok(()) => {
                tracing::debug!(interface = %iface_name, "removed netem");
            }
            Err(e) => {
                // An end with no impairment has no netem qdisc to remove.
                tracing::debug!(interface = %iface_name, error = %e, "no netem removed");
            }
        }
        return Ok(());
    }
    network::tc::update_netem(ifindex, impairment)
        .await
        .context(format!("Failed to apply netem on {iface_name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(delay: u32, loss_percent: f32) -> UpdateImpairmentRequest {
        UpdateImpairmentRequest {
            lab_id: "abc12345".to_owned(),
            link_index: 1,
            direction: ImpairmentDirection::Both,
            delay,
            jitter: 0,
            loss_percent,
            reorder_percent: 0.0,
            corrupt_percent: 0.0,
        }
    }

    #[test]
    fn a_requested_impairment_is_converted_to_microseconds() {
        let impairment = requested_impairment(&request(274_877, 12.5)).unwrap();
        assert_eq!(impairment.delay_us, 274_877_000);
        assert!((impairment.loss_percent - 12.5).abs() < f32::EPSILON);
    }

    #[test]
    fn delays_netem_cannot_hold_and_bad_percentages_are_invalid() {
        for bad in [
            request(274_878, 0.0),
            request(u32::MAX, 0.0),
            request(0, 100.5),
            request(0, -1.0),
        ] {
            let error = requested_impairment(&bad).expect_err("accepted");
            assert!(matches!(error, ServiceError::Invalid(_)), "{error}");
        }
    }
}
