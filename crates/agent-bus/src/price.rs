//! What the host's API responses cost, at the rates Claude Code itself charges at list, with no
//! I/O.
//!
//! Guests report Claude Code's own `costUSD`; the host's interactive transcripts carry tokens
//! but no cost, so the host prices them here. The rates were measured from Claude Code's own
//! figures (an exact linear fit over its per-model costs, five token kinds), not copied from a
//! price page. The 5-minute cache write is 1.25 times the input rate for every model; it was
//! only seen for Sonnet.
//!
//! A model missing from `RATES` is not priced: its rows keep their tokens and a null cost until
//! a rate is added and `host import --reprice` fills them. So is a response not at the
//! standard speed (fast mode). Adding a model is one line in `RATES` and one in its test.

/// The speed of a response priced at `RATES`.
pub const STANDARD_SPEED: &str = "standard";

/// The rates of one model, in thousandths of a US dollar per million tokens, so every rate and
/// every cost is a whole number until the last division.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rates {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
}

/// Thousandths of a dollar per million tokens, times the tokens, per dollar.
const NANODOLLARS: f64 = 1e9;
/// 2^32, to put the high half of a count back in place.
const TWO_TO_THE_32: f64 = 4_294_967_296.0;

/// The rates by model ID.
const RATES: &[(&str, Rates)] = &[
    ("claude-opus-5-5", rates(4_000, 20_000, 200, 8_000)),
    ("claude-sonnet-5-5", rates(2_000, 10_000, 100, 4_000)),
    ("claude-haiku-5-5", rates(100, 500, 10, 200)),
    ("claude-fable-5-2", rates(8_000, 40_000, 200, 16_000)),
];

/// The tokens of one API response, by kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

/// A model's rates from its input, output, cache read and one-hour cache write rates; the
/// 5-minute write is 1.25 times input.
const fn rates(input: u64, output: u64, cache_read: u64, cache_write_1h: u64) -> Rates {
    Rates {
        input,
        output,
        cache_read,
        cache_write_5m: input * 5 / 4,
        cache_write_1h,
    }
}

/// What `tokens` cost on `model` at `speed`, in US dollars; `None` when the model has no rate
/// or the speed is not standard.
pub fn cost(model: &str, speed: &str, tokens: &Tokens) -> Option<f64> {
    if speed != STANDARD_SPEED {
        return None;
    }
    let (_, rates) = RATES.iter().find(|(id, _)| *id == model)?;
    let nanodollars = [
        (tokens.input, rates.input),
        (tokens.output, rates.output),
        (tokens.cache_read, rates.cache_read),
        (tokens.cache_write_5m, rates.cache_write_5m),
        (tokens.cache_write_1h, rates.cache_write_1h),
    ]
    .iter()
    .try_fold(0_u128, |sum, (count, rate)| {
        sum.checked_add(u128::from(*count) * u128::from(*rate))
    })?;
    // Whole numbers up to 2^53 (about nine million dollars) are exact in an f64; u64 has no
    // lossless conversion, so it goes in two 32-bit halves.
    let nanodollars = u64::try_from(nanodollars).ok()?;
    let high = u32::try_from(nanodollars >> 32).ok()?;
    let low = u32::try_from(nanodollars & u64::from(u32::MAX)).ok()?;
    Some((f64::from(high) * TWO_TO_THE_32 + f64::from(low)) / NANODOLLARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, output: u64, cache_read: u64, five_minute: u64, one_hour: u64) -> Tokens {
        Tokens {
            input,
            output,
            cache_read,
            cache_write_5m: five_minute,
            cache_write_1h: one_hour,
        }
    }

    #[test]
    fn the_measured_opus_figure_is_reproduced() {
        let used = tokens(56, 18_340, 1_476_182, 0, 85_432);
        assert_eq!(
            cost("claude-opus-5-5", STANDARD_SPEED, &used),
            Some(1.345_716_4)
        );
    }

    #[test]
    fn every_model_has_its_five_rates() {
        let million = |kind: usize| {
            let mut counts = [0; 5];
            counts[kind] = 1_000_000;
            tokens(counts[0], counts[1], counts[2], counts[3], counts[4])
        };
        for (model, expected) in [
            ("claude-opus-5-5", [4.0, 20.0, 0.2, 5.0, 8.0]),
            ("claude-sonnet-5-5", [2.0, 10.0, 0.1, 2.5, 4.0]),
            ("claude-haiku-5-5", [0.1, 0.5, 0.01, 0.125, 0.2]),
            ("claude-fable-5-2", [8.0, 40.0, 0.2, 10.0, 16.0]),
        ] {
            for (kind, rate) in expected.iter().enumerate() {
                assert_eq!(
                    cost(model, STANDARD_SPEED, &million(kind)),
                    Some(*rate),
                    "{model} kind {kind}"
                );
            }
        }
        assert_eq!(RATES.len(), 4, "a new model needs its line here");
    }

    #[test]
    fn the_five_minute_write_is_one_and_a_quarter_times_input() {
        for (model, rates) in RATES {
            assert_eq!(rates.cache_write_5m * 4, rates.input * 5, "{model}");
        }
    }

    #[test]
    fn an_unknown_model_and_fast_mode_are_not_priced() {
        let used = tokens(1, 1, 1, 1, 1);
        assert_eq!(cost("claude-opus-9-9", STANDARD_SPEED, &used), None);
        assert_eq!(cost("claude-opus-5-5", "fast", &used), None);
        assert!(cost("claude-opus-5-5", STANDARD_SPEED, &used).is_some());
    }
}
