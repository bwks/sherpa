use anyhow::{Context, Result, anyhow};
use tracing::instrument;

use crate::daemon::state::AppState;
use network::tc::LinkImpairment;
use shared::data::{BridgeKind, UpdateImpairmentRequest, UpdateImpairmentResponse};
use shared::konst::TAP_PREFIX;

/// Update link impairment on a running P2p link
///
/// This function:
/// 1. Finds the link in the database by lab_id + link_index
/// 2. Validates it is a P2p link (impairment only works on P2p links)
/// 3. Applies TC netem impairment to both tap interfaces
/// 4. Updates the database record with the new impairment values
#[instrument(skip(state), fields(lab_id = %request.lab_id, link_index = %request.link_index))]
pub async fn update_impairment(
    request: UpdateImpairmentRequest,
    state: &AppState,
) -> Result<UpdateImpairmentResponse> {
    let lab_id = &request.lab_id;

    // Get all links for this lab and find the one with the matching index
    let lab = db::get_lab(&state.db, lab_id)
        .await
        .context(format!("Lab '{}' not found", lab_id))?;

    let lab_record_id = lab
        .id
        .ok_or_else(|| anyhow!("Lab '{}' missing record ID", lab_id))?;

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

    let impairment = LinkImpairment {
        delay_us: request.delay * 1000,
        jitter_us: request.jitter * 1000,
        loss_percent: request.loss_percent,
        reorder_percent: request.reorder_percent,
        corrupt_percent: request.corrupt_percent,
    };

    let is_zero = request.delay == 0
        && request.jitter == 0
        && request.loss_percent == 0.0
        && request.reorder_percent == 0.0
        && request.corrupt_percent == 0.0;

    // Resolve interface names
    let tap_a = format!("{}a{}-{}", TAP_PREFIX, request.link_index, lab_id);
    let tap_b = format!("{}b{}-{}", TAP_PREFIX, request.link_index, lab_id);

    // Get ifindex for both interfaces
    let ifindex_a = network::tap::get_ifindex(&tap_a)
        .await
        .context(format!("Failed to get ifindex for {}", tap_a))?;
    let ifindex_b = network::tap::get_ifindex(&tap_b)
        .await
        .context(format!("Failed to get ifindex for {}", tap_b))?;

    // Apply or remove netem on both interfaces
    if is_zero {
        // Remove impairment
        let _ = network::tc::remove_netem(ifindex_a as i32).await;
        let _ = network::tc::remove_netem(ifindex_b as i32).await;
        tracing::info!(
            lab_id = %lab_id,
            link_index = request.link_index,
            "Removed link impairment"
        );
    } else {
        // Apply/update impairment on both directions
        network::tc::update_netem(ifindex_a as i32, &impairment)
            .await
            .context(format!("Failed to apply netem on {}", tap_a))?;
        network::tc::update_netem(ifindex_b as i32, &impairment)
            .await
            .context(format!("Failed to apply netem on {}", tap_b))?;
        tracing::info!(
            lab_id = %lab_id,
            link_index = request.link_index,
            delay_ms = request.delay,
            jitter_ms = request.jitter,
            loss_percent = request.loss_percent,
            "Applied link impairment"
        );
    }

    // Update the database record (stored as microseconds)
    db_link.delay_us = request.delay * 1000;
    db_link.jitter_us = request.jitter * 1000;
    db_link.loss_percent = request.loss_percent;
    db_link.reorder_percent = request.reorder_percent;
    db_link.corrupt_percent = request.corrupt_percent;

    db::update_link(&state.db, db_link)
        .await
        .context("Failed to update link impairment in database")?;

    let message = if is_zero {
        format!(
            "Removed impairment from link {} in lab '{}'",
            request.link_index, lab_id
        )
    } else {
        format!(
            "Applied impairment to link {} in lab '{}': delay={}ms jitter={}ms loss={}% reorder={}% corrupt={}%",
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
