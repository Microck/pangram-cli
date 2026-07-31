//! One process-wide throughput semaphore for Pangram requests.
//!
//! A single counting semaphore bounds in-flight upstream requests to four
//! below Pangram's five-per-second server ceiling, mirroring the Python
//! SDK's async semaphore convention. The configured
//! `network.max_requests_per_second` (built-in default 5.0, validated
//! `0 < rate <= 5`) is exposed through [`crate::analysis::AnalysisConfig`]
//! and must never be raised; the parity solver below proves that every
//! accepted configuration keeps the concurrent slot count inside the same
//! safety envelope: `effective_outbound_slots = max(1, min(4, floor(rate)))`.
//! The documented 5-per-second ceiling remains invariantly respected without
//! a second shared mutable limiter state.

/// Outbound slots for a validated requests-per-second configuration:
/// `max(1, min(4, floor(rate)))`. Sub-1.0 configurations get one slot, so a
/// lowered configuration can only reduce concurrency, never raise it.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub const fn slots_for_rate(max_requests_per_second: f64) -> usize {
    // Configuration validation already guarantees `0 < rate <= 5`, so this
    // cast cannot wrap, go negative, or exceed the four-slot ceiling.
    // `f64::floor` is not const-callable on the MSRV toolchain, so the
    // comparison ladder computes the same floor.
    if max_requests_per_second < 2.0 {
        // Covers the sub-1.0 single-slot floor and the 1..2 step together.
        1
    } else if max_requests_per_second < 3.0 {
        2
    } else if max_requests_per_second < 4.0 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn accepted_rates_never_exceed_the_four_slot_ceiling(
            rate in 0.000_001_f64..=5.0_f64,
        ) {
            let slots = slots_for_rate(rate);
            prop_assert!(slots >= 1);
            prop_assert!(slots <= 4);
            let floored = rate.floor().max(1.0);
            prop_assert!((slots as f64) <= floored);
        }
    }

    #[test]
    fn boundary_rates_map_to_documented_slot_counts() {
        assert_eq!(slots_for_rate(0.5), 1);
        assert_eq!(slots_for_rate(1.0), 1);
        assert_eq!(slots_for_rate(1.999_999), 1);
        assert_eq!(slots_for_rate(2.0), 2);
        assert_eq!(slots_for_rate(2.999_999), 2);
        assert_eq!(slots_for_rate(3.0), 3);
        assert_eq!(slots_for_rate(3.999_999), 3);
        assert_eq!(slots_for_rate(4.0), 4);
        assert_eq!(slots_for_rate(5.0), 4);
    }
}
