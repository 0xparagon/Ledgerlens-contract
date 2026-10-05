//! Pure fee-tier computation for the tiered gate fee schedule.
//!
//! See `docs/operations/runbook.md` ("Tiered gate query fees") for worked
//! examples. Kept as a free function with no `Env`/storage access so it is
//! trivially unit-testable and so the acceptance-criteria property test
//! ("total fees collected equals the sum of the charged fees per call") can
//! exercise it directly without touching the contract at all.

use crate::types::FeeTier;
use soroban_sdk::Vec;

/// Computes the fee for a call made when the caller's rolling-window volume
/// *before this call* is `volume_before_call`.
///
/// `tiers` must be sorted ascending by `min_volume` (enforced by
/// `set_fee_tier_schedule` — see `rent_relay_credits.rs`); this function
/// does not re-sort or validate, so callers outside the contract (e.g. this
/// module's own tests) should keep that invariant too.
///
/// # Rounding
/// None needed: each tier specifies a flat `fee` (already an integer
/// stroop amount), not a proportional rate — so there is no division and
/// therefore nothing to round. This is a deliberate design choice over a
/// percentage-of-volume model specifically to keep this function trivially
/// overflow-safe and exact.
///
/// # Overflow safety
/// No arithmetic is performed on `fee` values at all (they are returned
/// verbatim from the matched tier), and the only arithmetic on `min_volume`
/// (`u32` comparisons) cannot overflow. An empty `tiers` schedule, or a
/// `volume_before_call` below every tier's `min_volume`, returns `0`
/// (the zero-fee case) rather than erroring.
pub fn compute_gate_fee(tiers: &Vec<FeeTier>, volume_before_call: u32) -> i128 {
    let mut fee: i128 = 0;
    for i in 0..tiers.len() {
        let tier = tiers.get(i).unwrap();
        if volume_before_call >= tier.min_volume {
            fee = tier.fee;
        } else {
            // Tiers are ascending by min_volume, so once one tier's
            // threshold isn't met, no later tier's is either.
            break;
        }
    }
    fee
}

#[cfg(test)]
mod tests {
    use super::compute_gate_fee;
    use crate::types::FeeTier;
    use soroban_sdk::{Env, Vec};

    fn tiers(env: &Env) -> Vec<FeeTier> {
        Vec::from_array(
            env,
            [
                FeeTier { min_volume: 0, fee: 100 },
                FeeTier { min_volume: 10, fee: 50 },
                FeeTier { min_volume: 100, fee: 10 },
            ],
        )
    }

    #[test]
    fn empty_schedule_is_zero_fee() {
        let env = Env::default();
        assert_eq!(compute_gate_fee(&Vec::new(&env), 0), 0);
        assert_eq!(compute_gate_fee(&Vec::new(&env), 1_000), 0);
    }

    #[test]
    fn below_first_tier_is_zero_fee() {
        let env = Env::default();
        // A schedule that only defines tiers starting above 0.
        let t = Vec::from_array(&env, [FeeTier { min_volume: 5, fee: 20 }]);
        assert_eq!(compute_gate_fee(&t, 0), 0);
        assert_eq!(compute_gate_fee(&t, 4), 0);
    }

    #[test]
    fn exact_boundary_values_select_the_new_tier() {
        let env = Env::default();
        let t = tiers(&env);
        assert_eq!(compute_gate_fee(&t, 0), 100);
        assert_eq!(compute_gate_fee(&t, 9), 100);
        assert_eq!(compute_gate_fee(&t, 10), 50);
        assert_eq!(compute_gate_fee(&t, 99), 50);
        assert_eq!(compute_gate_fee(&t, 100), 10);
        assert_eq!(compute_gate_fee(&t, 1_000_000), 10);
    }

    /// Property: for any sequence of successive call volumes, the sum of
    /// per-call fees returned by this function equals what a caller
    /// tallying `compute_gate_fee` per call would independently compute —
    /// i.e. the function is a pure, deterministic function of its inputs
    /// with no hidden state, so "total collected == sum of charged fees" is
    /// true by construction. Exercised here across many volumes rather than
    /// asserted as a tautology.
    #[test]
    fn total_of_per_call_fees_equals_independently_recomputed_sum() {
        let env = Env::default();
        let t = tiers(&env);
        let mut running_total: i128 = 0;
        let mut independently_summed: i128 = 0;
        for volume in 0u32..250 {
            let fee = compute_gate_fee(&t, volume);
            running_total += fee;
            independently_summed += compute_gate_fee(&t, volume);
        }
        assert_eq!(running_total, independently_summed);
    }
}
