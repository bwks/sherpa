use anyhow::{Context, Result, anyhow};
use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use shared::data::{
    BridgeKind, ImpairmentDirection, LinkImpairment, UpdateImpairmentRequest,
    UpdateImpairmentResponse,
};

/// Update link impairment on a running P2p link
///
/// This function:
/// 1. Finds the link in the database by lab_id + link_index
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

    // Get all links for this lab and find the one with the matching index
    let lab = require_lab_access(ctx, &state.db, lab_id).await?;

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

    // Traffic from A to B leaves the host through B's end, so its netem goes on tap_b.
    let (to_b, to_a) = match request.direction {
        ImpairmentDirection::Both => (true, true),
        ImpairmentDirection::SrcToDst => (true, false),
        ImpairmentDirection::DstToSrc => (false, true),
    };
    if to_b {
        set_netem(&db_link.tap_b, &impairment).await?;
        db_link.impairment_a_to_b = impairment;
    }
    if to_a {
        set_netem(&db_link.tap_a, &impairment).await?;
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

    db::update_link(&state.db, db_link)
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

/// Replace the netem impairment on a host end, or remove it when `impairment` is clear.
#[instrument(skip(impairment), fields(%iface_name), level = "debug")]
async fn set_netem(iface_name: &str, impairment: &LinkImpairment) -> Result<()> {
    let ifindex = network::tap::get_ifindex(iface_name)
        .await
        .context(format!("Failed to get ifindex for {iface_name}"))?;
    if impairment.is_clear() {
        match network::tc::remove_netem(ifindex as i32).await {
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
    network::tc::update_netem(ifindex as i32, impairment)
        .await
        .context(format!("Failed to apply netem on {iface_name}"))
}
