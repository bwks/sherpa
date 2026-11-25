use anyhow::{Result, ensure};

use crate::SharedError;

/// The longest delay or jitter netem holds, in milliseconds. netem counts them in 32-bit
/// PSCHED ticks at 15.625 MHz, so `u32::MAX` ticks are just over 274.877 seconds.
pub(crate) const MAX_IMPAIRMENT_DELAY_MS: u32 = 274_877;

/// Check a link impairment as the manifest and the live update API give it: delay and jitter
/// in milliseconds, loss, reorder and corruption in percent.
///
/// # Errors
///
/// `SharedError::ImpairmentDelayTooLong` when a delay is longer than netem can hold, and
/// `SharedError::ImpairmentPercentOutOfRange` when a percentage is not between 0 and 100, each
/// carrying the field and its value.
pub fn validate_link_impairment(
    delay_ms: u32,
    jitter_ms: u32,
    loss_percent: f64,
    reorder_percent: f64,
    corrupt_percent: f64,
) -> Result<()> {
    for (field, value) in [("delay", delay_ms), ("jitter", jitter_ms)] {
        ensure!(
            value <= MAX_IMPAIRMENT_DELAY_MS,
            SharedError::ImpairmentDelayTooLong { field, value }
        );
    }
    for (field, value) in [
        ("loss_percent", loss_percent),
        ("reorder_percent", reorder_percent),
        ("corrupt_percent", corrupt_percent),
    ] {
        ensure!(
            (0.0..=100.0).contains(&value),
            SharedError::ImpairmentPercentOutOfRange { field, value }
        );
    }
    Ok(())
}
