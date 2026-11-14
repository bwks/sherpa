use anyhow::{Context, Result, anyhow};
use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use crate::services::p2p_links::{KernelHost, LinkHost};
use shared::data::{
    BridgeKind, DbLink, ImpairmentDirection, LinkImpairment, UpdateImpairmentRequest,
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

    let db_link = links
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

    let host = KernelHost::new(&state.docker, &state.qemu);
    change_impairment(
        &host,
        &db_link,
        request.direction,
        &impairment,
        |link| async move {
            let link_record = link.id.as_ref().context("Link record has no id")?;
            db::update_link_impairments(
                &state.db,
                link_record,
                &link.impairment_a_to_b,
                &link.impairment_b_to_a,
            )
            .await
            .context("Failed to update link impairment in database")?;
            Ok(())
        },
    )
    .await?;
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

/// Change `link`'s impairment in the `direction` asked to `impairment`: in the kernel, then
/// with `record`, which stores the changed link. Returns the changed link.
///
/// The kernel is changed first and put back when anything after it fails, so a failed request
/// leaves the kernel and the database at the old value together. The kernel is the usual
/// failure, and it then fails before the database is touched. If putting the kernel back fails
/// too, the error says so, and the link's next re-attach syncs the kernel to the database.
async fn change_impairment<F, Fut>(
    host: &impl LinkHost,
    link: &DbLink,
    direction: ImpairmentDirection,
    impairment: &LinkImpairment,
    record: F,
) -> Result<DbLink>
where
    F: FnOnce(DbLink) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    // Traffic from A to B leaves the host through B's end, so its netem goes on tap_b.
    let (to_b, to_a) = match direction {
        ImpairmentDirection::Both => (true, true),
        ImpairmentDirection::SrcToDst => (true, false),
        ImpairmentDirection::DstToSrc => (false, true),
    };
    let mut changed = link.clone();
    // The ends changed so far, each with the value to put back.
    let mut done: Vec<(&str, LinkImpairment)> = vec![];
    let steps = [
        (to_b, link.tap_b.as_str(), link.impairment_a_to_b),
        (to_a, link.tap_a.as_str(), link.impairment_b_to_a),
    ];
    for (wanted, iface, old) in steps {
        if !wanted {
            continue;
        }
        if let Err(error) = set_netem(host, iface, impairment).await {
            return Err(put_back(host, &done, error).await);
        }
        done.push((iface, old));
    }
    if to_b {
        changed.impairment_a_to_b = *impairment;
    }
    if to_a {
        changed.impairment_b_to_a = *impairment;
    }
    if let Err(error) = record(changed.clone()).await {
        return Err(put_back(host, &done, error).await);
    }
    Ok(changed)
}

/// Put each end in `done` back to its old impairment after `error`, and return the error,
/// saying whether the kernel was put back.
async fn put_back(
    host: &impl LinkHost,
    done: &[(&str, LinkImpairment)],
    error: anyhow::Error,
) -> anyhow::Error {
    let mut not_restored = vec![];
    for (iface, old) in done {
        if let Err(restore) = set_netem(host, iface, old).await {
            tracing::error!(interface = %iface, error = %restore, "Failed to put the impairment back");
            not_restored.push(*iface);
        }
    }
    if not_restored.is_empty() {
        error
    } else {
        error.context(format!(
            "The impairment on {} could not be put back; it is set to the recorded value when \
             the link is next re-attached",
            not_restored.join(", ")
        ))
    }
}

/// Set the netem impairment on a host end, or remove it when `impairment` is clear. Only an end
/// with no netem qdisc counts as clear without a removal; a removal that fails is an error.
#[instrument(skip(host, impairment), fields(%iface_name), level = "debug")]
async fn set_netem(
    host: &impl LinkHost,
    iface_name: &str,
    impairment: &LinkImpairment,
) -> Result<()> {
    let ifindex = host
        .ifindex(iface_name)
        .await?
        .with_context(|| format!("Interface {iface_name} does not exist; is its node running?"))?;
    host.sync_netem(ifindex, impairment)
        .await
        .with_context(|| format!("Failed to set the impairment on {iface_name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::Cell;

    use anyhow::bail;
    use shared::data::RecordId;

    use crate::services::p2p_links::fake::FakeHost;

    fn request(delay: u32, loss_percent: f64) -> UpdateImpairmentRequest {
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
        assert!((impairment.loss_percent - 12.5).abs() < f64::EPSILON);
    }

    fn link() -> DbLink {
        DbLink {
            id: None,
            index: 0,
            kind: BridgeKind::P2p,
            node_a: RecordId::new("node", "vm1"),
            node_b: RecordId::new("node", "vm2"),
            int_a: "eth1".to_string(),
            int_b: "eth1".to_string(),
            lab: RecordId::new("lab", "lab"),
            tap_a: "tpa0".to_string(),
            tap_b: "tpb0".to_string(),
            impairment_a_to_b: delay(5),
            impairment_b_to_a: delay(5),
        }
    }

    fn delay(ms: u32) -> LinkImpairment {
        LinkImpairment {
            delay_us: ms * 1000,
            ..LinkImpairment::default()
        }
    }

    /// A host with the link's ends, each with the link's impairment of 5 ms.
    fn host() -> FakeHost {
        let host = FakeHost::default();
        host.make(&["tpa0", "tpb0"]);
        for tap in ["tpa0", "tpb0"] {
            let index = host.index(tap).unwrap();
            host.netem.borrow_mut().insert(index, delay(5));
        }
        host
    }

    #[tokio::test]
    async fn a_clear_that_the_kernel_refuses_is_not_recorded() {
        // M25.3: a failed removal used to count as "nothing to remove", and the database was
        // set clear while the kernel kept the qdisc.
        let mut host = host();
        host.failing_netem.insert("tpb0".to_string());
        let recorded = Cell::new(false);
        let result = change_impairment(
            &host,
            &link(),
            ImpairmentDirection::SrcToDst,
            &LinkImpairment::default(),
            |_| async {
                recorded.set(true);
                Ok(())
            },
        )
        .await;
        assert!(result.is_err());
        assert!(!recorded.get(), "the database must keep the impairment");
        assert_eq!(host.netem_on("tpb0"), Some(delay(5)));
    }

    #[tokio::test]
    async fn a_clear_with_nothing_to_remove_is_recorded() {
        let host = FakeHost::default();
        host.make(&["tpa0", "tpb0"]);
        let changed = change_impairment(
            &host,
            &link(),
            ImpairmentDirection::Both,
            &LinkImpairment::default(),
            |_| async { Ok(()) },
        )
        .await
        .unwrap();
        assert!(changed.impairment_a_to_b.is_clear());
        assert!(changed.impairment_b_to_a.is_clear());
    }

    #[tokio::test]
    async fn a_change_the_database_refuses_is_taken_back_from_the_kernel() {
        // M25.4: the kernel used to keep the new value while the database kept the old one.
        let host = host();
        let result = change_impairment(
            &host,
            &link(),
            ImpairmentDirection::Both,
            &delay(50),
            |_| async { bail!("database is locked") },
        )
        .await;
        let error = result.unwrap_err();
        assert!(
            format!("{error:#}").contains("database is locked"),
            "{error:#}"
        );
        assert_eq!(host.netem_on("tpa0"), Some(delay(5)));
        assert_eq!(host.netem_on("tpb0"), Some(delay(5)));
    }

    #[tokio::test]
    async fn a_change_the_kernel_refuses_halfway_is_taken_back() {
        let mut host = host();
        host.failing_netem.insert("tpa0".to_string());
        let recorded = Cell::new(false);
        let result = change_impairment(
            &host,
            &link(),
            ImpairmentDirection::Both,
            &delay(50),
            |_| async {
                recorded.set(true);
                Ok(())
            },
        )
        .await;
        assert!(result.is_err());
        assert!(!recorded.get());
        assert_eq!(host.netem_on("tpb0"), Some(delay(5)));
    }

    #[tokio::test]
    async fn a_change_is_set_on_the_end_each_direction_leaves_through() {
        let host = host();
        let changed = change_impairment(
            &host,
            &link(),
            ImpairmentDirection::SrcToDst,
            &delay(50),
            |_| async { Ok(()) },
        )
        .await
        .unwrap();
        assert_eq!(changed.impairment_a_to_b, delay(50));
        assert_eq!(changed.impairment_b_to_a, delay(5));
        assert_eq!(host.netem_on("tpb0"), Some(delay(50)));
        assert_eq!(host.netem_on("tpa0"), Some(delay(5)));
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
