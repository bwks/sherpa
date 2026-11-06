use anyhow::{Result, ensure};

/// The longest delay or jitter netem holds, in milliseconds. netem counts them in 32-bit
/// PSCHED ticks at 15.625 MHz, so `u32::MAX` ticks are just over 274.877 seconds.
const MAX_IMPAIRMENT_DELAY_MS: u32 = 274_877;

/// Check a link impairment as the manifest and the live update API give it: delay and jitter
/// in milliseconds, loss, reorder and corruption in percent.
///
/// # Errors
///
/// Fails when a delay is longer than netem can hold, or a percentage is not between 0 and 100.
pub fn validate_link_impairment(
    delay_ms: u32,
    jitter_ms: u32,
    loss_percent: f32,
    reorder_percent: f32,
    corrupt_percent: f32,
) -> Result<()> {
    for (field, value) in [("delay", delay_ms), ("jitter", jitter_ms)] {
        ensure!(
            value <= MAX_IMPAIRMENT_DELAY_MS,
            "Impairment {field} of {value} ms is longer than the {MAX_IMPAIRMENT_DELAY_MS} ms netem can hold"
        );
    }
    for (field, value) in [
        ("loss_percent", loss_percent),
        ("reorder_percent", reorder_percent),
        ("corrupt_percent", corrupt_percent),
    ] {
        ensure!(
            (0.0..=100.0).contains(&value),
            "Impairment {field} must be between 0 and 100, not {value}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impairments_within_netem_limits_are_accepted() {
        validate_link_impairment(0, 0, 0.0, 0.0, 0.0).unwrap();
        validate_link_impairment(
            MAX_IMPAIRMENT_DELAY_MS,
            MAX_IMPAIRMENT_DELAY_MS,
            100.0,
            100.0,
            100.0,
        )
        .unwrap();
    }

    #[test]
    fn the_longest_delay_fits_netem_ticks() {
        let ticks = u64::from(MAX_IMPAIRMENT_DELAY_MS) * 1_000 * 15_625 / 1_000;
        assert!(ticks <= u64::from(u32::MAX));
        let next = u64::from(MAX_IMPAIRMENT_DELAY_MS + 1) * 1_000 * 15_625 / 1_000;
        assert!(next > u64::from(u32::MAX));
    }

    #[test]
    fn delays_netem_cannot_hold_are_refused() {
        let error = validate_link_impairment(MAX_IMPAIRMENT_DELAY_MS + 1, 0, 0.0, 0.0, 0.0)
            .unwrap_err()
            .to_string();
        assert!(error.contains("delay"), "{error}");
        let error = validate_link_impairment(0, u32::MAX, 0.0, 0.0, 0.0)
            .unwrap_err()
            .to_string();
        assert!(error.contains("jitter"), "{error}");
    }

    #[test]
    fn percentages_outside_0_to_100_are_refused() {
        for (loss, reorder, corrupt, field) in [
            (-5.0, 0.0, 0.0, "loss_percent"),
            (150.0, 0.0, 0.0, "loss_percent"),
            (0.0, f32::NAN, 0.0, "reorder_percent"),
            (0.0, 0.0, 100.1, "corrupt_percent"),
        ] {
            let error = validate_link_impairment(0, 0, loss, reorder, corrupt)
                .unwrap_err()
                .to_string();
            assert!(error.contains(field), "{error}");
        }
    }
}
