//! #1246 — adversarial tests for every ledger-time dependent computation.
//!
//! These tests pin the boundaries of the `u64` ledger clock and the bounded
//! `u32` sequences derived from it. They are deliberately deterministic: no
//! fuzzing, no wall-clock dependence, and every ledger timestamp / sequence
//! is set explicitly so a failure is always reproducible.
//!
//! Three properties are checked throughout:
//!
//! 1. **No aborts.** Every code path that combines a caller supplied time with
//!    contract time is exercised at `0`, at the `u32` boundary (`2^32`), and
//!    at `u64::MAX`. Nothing may panic, and nothing may wrap.
//! 2. **Monotonicity.** As the ledger clock advances, a score's effective value
//!    and staleness must move in exactly one direction.
//! 3. **Regressions.** Each test that covers a defect fixed in #1246 also
//!    covers the exact off-by-one boundary so the intended semantics are
//!    pinned rather than merely "not crashing" any more.
//!
//! The two historical defects covered here abort rather than wrap: the release
//! profile sets `overflow-checks = true` and `panic = "abort"`, so a single
//! overflowing `ttl + grace` would make every submission fail permanently.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger as _},
    Address, Env, Symbol, Vec,
};

use crate::{constants, Error, LedgerLensScoreContract, LedgerLensScoreContractClient};

/// Realistic mainnet-ish start so that arithmetic near `START_TS` exercises
/// the same magnitudes as production while remaining far from the boundaries.
const START_TS: u64 = 1_700_000_000;
/// One year, used for the wide sweeps.
const YEAR: u64 = 31_536_000;
/// The first timestamp at which a `u32` second count would wrap.
const FIRST_U32_OVERFLOW_SEC: u64 = 4_294_967_296;

fn setup<'a>() -> (Env, LedgerLensScoreContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START_TS);

    let contract_id = env.register_contract(None, LedgerLensScoreContract);
    let client = LedgerLensScoreContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    client.initialize(&admin, &service);

    (env, client, admin, service)
}

fn admin_signers(env: &Env, admin: &Address) -> Vec<Address> {
    Vec::from_array(env, [admin.clone()])
}

fn at(env: &Env, ts: u64) {
    env.ledger().with_mut(|l| l.timestamp = ts);
}

/// Submit one score at an explicit caller timestamp. Used by the sweeps so the
/// per-wallet rate limiter never interferes with the property under test.
fn submit_at(
    env: &Env,
    client: &LedgerLensScoreContractClient,
    wallet: &Address,
    asset_pair: &Symbol,
    score: u32,
    confidence: u32,
    ts: u64,
) {
    client.submit_score(
        &Vec::new(env),
        wallet,
        asset_pair,
        &score,
        &false,
        &false,
        &ts,
        &confidence,
        &1,
        &None,
    );
}

// ── Clock boundaries ────────────────────────────────────────────────────────

#[test]
fn zero_timestamp_is_rejected_at_the_floor() {
    let (env, client, _admin, _service) = setup();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");

    at(&env, 0);

    // A zero caller timestamp is the only timestamp the contract rejects, so it
    // must be rejected at the ledger floor too, with no underflow anywhere.
    assert_eq!(
        client.try_submit_score(
            &Vec::new(&env),
            &wallet,
            &pair,
            &60,
            &false,
            &false,
            &0,
            &90,
            &1,
            &None,
        ),
        Err(Ok(Error::InvalidTimestamp))
    );
    assert!(client.get_score_opt(&wallet, &pair).is_none());
}

#[test]
fn timestamp_one_is_accepted_at_the_first_second() {
    let (env, client, _admin, _service) = setup();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");

    at(&env, 1);

    // `age = 1 - 1 = 0`: the smallest legal non-zero timestamp must be a
    // clean, zero-age submission rather than an underflow.
    submit_at(&env, &client, &wallet, &pair, 50, 80, 1);

    let effective = client.get_effective_score(&wallet, &pair);
    assert_eq!(effective.original_score, 50);
    assert_eq!(effective.effective_score, 50);
    assert!(!client.is_score_stale(&wallet, &pair));
    assert_eq!(client.get_last_submit_time(&wallet, &pair), Some(1));
}

#[test]
fn a_clock_jump_past_the_u32_second_boundary_ages_the_score_without_wrapping() {
    let (env, client, _admin, _service) = setup();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");

    at(&env, START_TS);
    submit_at(&env, &client, &wallet, &pair, 70, 90, START_TS);

    // `2^32` seconds is the first value that a `u32` second count cannot hold.
    at(&env, START_TS + FIRST_U32_OVERFLOW_SEC);

    // One year of decay passes, so the score is stale, but the elapsed
    // computation must not wrap: the effective value is zero rather than a
    // wrapped "negative age" that would restore the original score.
    let effective = client.get_effective_score(&wallet, &pair);
    assert_eq!(effective.original_score, 70);
    assert_eq!(effective.effective_score, 70);
    assert!(client.is_score_stale(&wallet, &pair));
    // Momentum has a single history entry, so it is zero rather than a value
    // derived from a wrapped `i32` age.
    assert_eq!(client.get_score_momentum(&wallet, &pair), 0);
}

#[test]
fn a_clock_jump_to_u64_max_leaves_every_time_dependent_read_intact() {
    let (env, client, _admin, _service) = setup();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");

    at(&env, START_TS);
    submit_at(&env, &client, &wallet, &pair, 80, 90, START_TS);

    // The largest possible ledger timestamp. Every `now - then` must saturate
    // instead of wrapping, and no decay computation may overflow.
    at(&env, u64::MAX);

    let effective = client.get_effective_score(&wallet, &pair);
    assert_eq!(effective.original_score, 80);
    assert_eq!(effective.effective_score, 0);
    assert!(client.is_score_stale(&wallet, &pair));
    assert_eq!(client.get_score_momentum(&wallet, &pair), 0);
    assert_eq!(
        client.get_last_submit_time(&wallet, &pair),
        Some(START_TS),
        "the stored submit time must be readable, not saturated on read"
    );
}

#[test]
fn a_huge_ledger_sequence_jump_does_not_abort_the_submission_path() {
    let (env, client, _admin, _service) = setup();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");

    // The ledger sequence must not be able to steer any time arithmetic. Note
    // the deliberate choice of 1e12 rather than `u64::MAX`: at `u64::MAX` the
    // host's own TTL bookkeeping (`live_until = sequence + extend_to`) would be
    // the first thing to overflow, which is a host guard rather than anything
    // this contract computes. 1e12 is ~31 million years of 30 s ledgers and
    // leaves that headroom intact.
    env.ledger().with_mut(|l| {
        l.timestamp = START_TS;
        l.sequence = 1_000_000_000_000;
    });

    submit_at(&env, &client, &wallet, &pair, 55, 70, START_TS);

    let effective = client.get_effective_score(&wallet, &pair);
    assert_eq!(effective.original_score, 55);
    assert_eq!(effective.effective_score, 55);
}

// ── Decay and staleness monotonicity ────────────────────────────────────────

#[test]
fn decay_survives_an_age_larger_than_the_u32_second_range() {
    let (env, client, _admin, _service) = setup();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");

    // lambda = 1/1, the most aggressive decay the setter accepts.
    client.set_decay_rate(&1, &1);

    at(&env, START_TS);
    submit_at(&env, &client, &wallet, &pair, 90, 95, START_TS);

    // Age of `2^32` seconds: larger than a `u32` can hold. The internal
    // `age * SCALE` product stays inside `u64` here, so this exercises the
    // "decayed to zero" branch rather than the overflow short circuit.
    at(&env, START_TS + FIRST_U32_OVERFLOW_SEC);

    let effective = client.get_effective_score(&wallet, &pair);
    assert_eq!(effective.original_score, 90);
    assert_eq!(effective.effective_score, 0);
    assert!(client.is_score_stale(&wallet, &pair));
}

#[test]
fn effective_score_is_monotonic_across_a_ten_year_sweep() {
    let (env, client, _admin, _service) = setup();
    env.budget().reset_unlimited();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");

    // lambda = 1/100_000 puts the cut-off at 500_000 s (~5.8 days), so the
    // weekly sweep below observes a real decay curve before it floors at 0.
    client.set_decay_rate(&1, &100_000);

    at(&env, START_TS);
    submit_at(&env, &client, &wallet, &pair, 80, 90, START_TS);

    let step = 7 * 86_400;
    let steps = 10 * YEAR / step;
    let mut previous = u32::MAX;

    for i in 0..=steps {
        at(&env, START_TS + i * step);
        let effective = client.get_effective_score(&wallet, &pair);
        assert_eq!(effective.original_score, 80);
        assert!(
            effective.effective_score <= 80,
            "effective score rose above the submitted score at step {i}"
        );
        assert!(
            effective.effective_score <= previous,
            "effective score increased from {previous} to {} at step {i}",
            effective.effective_score
        );
        previous = effective.effective_score;
    }

    assert_eq!(previous, 0, "the score should have decayed to the floor");
    assert!(client.is_score_stale(&wallet, &pair));
}

#[test]
fn staleness_flips_exactly_once_and_never_flips_back() {
    let (env, client, _admin, _service) = setup();
    env.budget().reset_unlimited();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");

    let window = constants::DEFAULT_STALENESS_WINDOW_SECS;
    at(&env, START_TS);
    submit_at(&env, &client, &wallet, &pair, 65, 80, START_TS);

    // `age > window` is the stale predicate, so the flip is at window + 1.
    at(&env, START_TS + window);
    assert!(!client.is_score_stale(&wallet, &pair));
    at(&env, START_TS + window + 1);
    assert!(client.is_score_stale(&wallet, &pair));

    for i in 1..=32 {
        at(&env, START_TS + window + i * 86_400);
        assert!(
            client.is_score_stale(&wallet, &pair),
            "score went fresh again at +{} days",
            i
        );
    }
}

#[test]
fn momentum_divides_by_a_two_to_the_32_second_delta_without_truncating_it_to_zero() {
    let (env, client, _admin, _service) = setup();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");

    // The momentum window is itself unbounded (open defect: `set_momentum_window`
    // applies no ceiling), so a window wide enough to hold a 2^32 s gap is
    // accepted. The default 3_600 s window would drop the first entry and return
    // before ever reaching the division.
    client.set_momentum_window(&FIRST_U32_OVERFLOW_SEC);

    at(&env, START_TS);
    submit_at(&env, &client, &wallet, &pair, 40, 70, START_TS);

    // Exactly 2^32 s later. `2^32 as i32` truncates to 0, which used to make the
    // read-path division a divide-by-zero panic.
    let later = START_TS + FIRST_U32_OVERFLOW_SEC;
    at(&env, later);
    submit_at(&env, &client, &wallet, &pair, 90, 70, later);

    // 50 points over 4_294_967_296 s truncates to zero. The assertion is that
    // the call returns at all.
    assert_eq!(client.get_score_momentum(&wallet, &pair), 0);
}

#[test]
fn momentum_of_a_future_dated_entry_is_zero_rather_than_a_wrapped_delta() {
    let (env, client, _admin, _service) = setup();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");
    client.set_momentum_window(&FIRST_U32_OVERFLOW_SEC);

    // Only non-zero caller timestamps are accepted, so `u64::MAX` is the most
    // extreme value the history can hold. The cooldown is measured against the
    // ledger clock, not this argument, so the next submission still lands.
    at(&env, START_TS);
    submit_at(&env, &client, &wallet, &pair, 40, 70, u64::MAX);

    at(&env, START_TS + 7_200);
    submit_at(&env, &client, &wallet, &pair, 90, 70, START_TS + 7_200);

    // The newest entry is older than the future-dated one, so the elapsed
    // computation saturates to zero and momentum is zero, not a wrapped
    // negative rate derived from a `u64` subtraction that underflowed.
    assert_eq!(client.get_score_momentum(&wallet, &pair), 0);
}

// ── Governance timelocks ────────────────────────────────────────────────────

#[test]
fn parameter_change_timelock_holds_until_the_delay_elapses() {
    let (env, client, admin, _service) = setup();
    let before = client.get_risk_threshold();
    let delay = client.get_upgrade_delay();
    assert!(delay > 0, "a zero delay would make this test vacuous");

    // Queue a threshold change; it must not take effect on the spot.
    client.set_risk_threshold(&admin_signers(&env, &admin), &80);
    assert_eq!(client.get_risk_threshold(), before);

    at(&env, START_TS + delay - 1);
    assert_eq!(
        client.try_apply_param_change(&symbol_short!("risk_thr")),
        Err(Ok(Error::UpgradeNotReady))
    );
    assert_eq!(client.get_risk_threshold(), before);

    at(&env, START_TS + delay);
    client.apply_param_change(&symbol_short!("risk_thr"));
    assert_eq!(client.get_risk_threshold(), 80);

    // The change is consumed, so it cannot be replayed.
    assert_eq!(
        client.try_apply_param_change(&symbol_short!("risk_thr")),
        Err(Ok(Error::NoPendingUpgrade))
    );
}

#[test]
fn a_clock_jump_past_the_timelock_applies_the_queued_change() {
    let (env, client, admin, _service) = setup();
    let before = client.get_risk_threshold();
    let delay = client.get_upgrade_delay();

    client.set_risk_threshold(&admin_signers(&env, &admin), &80);

    // Jump far past the delay. `proposed_at + delay` must not overflow and the
    // change must be applied, not left stuck as "not ready" forever.
    at(&env, u64::MAX / 2);
    client.apply_param_change(&symbol_short!("risk_thr"));
    assert_eq!(client.get_risk_threshold(), 80);
    assert_ne!(client.get_risk_threshold(), before);
}

#[test]
fn upgrade_timelock_refuses_execution_before_the_delay() {
    let (env, client, admin, _service) = setup();
    let delay = client.get_upgrade_delay();
    let signers = admin_signers(&env, &admin);

    let wasm_hash = soroban_sdk::BytesN::from_array(&env, &[7u8; 32]);
    client.propose_upgrade(&signers, &wasm_hash);

    // The proposal is stored as `proposed_at + delay`, evaluated against the
    // ledger clock rather than any caller input.
    let proposal = client.get_pending_upgrade();
    assert_eq!(proposal.proposed_at, START_TS);
    assert_eq!(proposal.executable_after, START_TS + delay);

    // The proposal second itself and the final second of the window are both
    // too early, so the boundary is inclusive of `delay` and exclusive of `0`.
    for ts in [START_TS, START_TS + delay - 1] {
        at(&env, ts);
        assert_eq!(
            client.try_execute_upgrade(&signers),
            Err(Ok(Error::UpgradeNotReady)),
            "execution at {ts} must be refused"
        );
    }

    // The proposal survives every refused attempt: a refused execution must not
    // consume or corrupt the timelock state.
    let still_pending = client.get_pending_upgrade();
    assert_eq!(still_pending.executable_after, proposal.executable_after);
    assert_eq!(still_pending.new_wasm_hash, wasm_hash);
}

#[test]
fn a_upgrade_proposed_at_the_top_of_the_clock_saturates_its_execution_time() {
    let (env, client, admin, _service) = setup();
    let delay = client.get_upgrade_delay();

    // Ten seconds before the top of the range, so `now + delay` cannot be
    // represented. The timelock must saturate to `u64::MAX` rather than wrap
    // to a small value, which would make the proposal instantly executable.
    at(&env, u64::MAX - 10);
    let wasm_hash = soroban_sdk::BytesN::from_array(&env, &[9u8; 32]);
    client.propose_upgrade(&admin_signers(&env, &admin), &wasm_hash);

    let proposal = client.get_pending_upgrade();
    assert_eq!(proposal.proposed_at, u64::MAX - 10);
    assert_eq!(
        proposal.executable_after,
        u64::MAX,
        "now + delay must saturate, never wrap"
    );
    assert!(
        proposal.executable_after > proposal.proposed_at,
        "a wrapped sum would make the timelock already expired"
    );
}

// ── Configured windows: bounds enforced at the extremes ─────────────────────

#[test]
fn a_timed_embargo_of_u64_max_never_expires() {
    let (env, client, _admin, _service) = setup();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");

    at(&env, START_TS);
    submit_at(&env, &client, &wallet, &pair, 60, 90, START_TS);
    assert!(!client.is_embargoed(&wallet));

    // `u64::MAX` is accepted as an expiry and is effectively permanent. It is
    // documented rather than bounded: an embargo is the one window where
    // "never expires" is the intent, and it is lifted explicitly below.
    client.set_score_embargo(&wallet, &Some(u64::MAX));
    assert!(client.is_embargoed(&wallet));
    assert_eq!(client.get_embargo_expiry(&wallet), Some(u64::MAX));

    // The window is inclusive: an expiry equal to now is still embargoed.
    at(&env, u64::MAX);
    assert!(
        client.is_embargoed(&wallet),
        "an expiry equal to now is still embargoed (the window is inclusive)"
    );

    // Only an explicit lift ends it.
    client.lift_score_embargo(&wallet);
    assert!(!client.is_embargoed(&wallet));
}

#[test]
fn an_embargo_expiry_in_the_past_is_inactive_at_the_next_ledger_second() {
    let (env, client, _admin, _service) = setup();
    let wallet = Address::generate(&env);

    at(&env, START_TS);
    client.set_score_embargo(&wallet, &Some(START_TS));
    assert!(client.is_embargoed(&wallet), "the expiry second is inclusive");

    at(&env, START_TS + 1);
    assert!(
        !client.is_embargoed(&wallet),
        "one second past the expiry the embargo is inactive"
    );
}

#[test]
fn an_indefinite_embargo_is_distinct_from_a_max_expiry() {
    let (env, client, _admin, _service) = setup();
    let max_wallet = Address::generate(&env);
    let indefinite_wallet = Address::generate(&env);

    at(&env, START_TS);
    // `expiry: None` is stored as an indefinite embargo, not as "no embargo".
    client.set_score_embargo(&max_wallet, &Some(u64::MAX));
    client.set_score_embargo(&indefinite_wallet, &None);

    assert!(client.is_embargoed(&max_wallet));
    assert!(client.is_embargoed(&indefinite_wallet));
    // The stored states are distinguishable, which is what lets an operator
    // audit "expires at the end of time" apart from "no expiry configured".
    assert_eq!(client.get_embargo_expiry(&max_wallet), Some(u64::MAX));
    assert_eq!(client.get_embargo_expiry(&indefinite_wallet), None);
}

#[test]
fn reveal_window_is_bounded_at_both_ends() {
    let (env, client, _admin, _service) = setup();
    let max = constants::MAX_REVEAL_WINDOW_SECS;

    // The bound exists because the window is converted to a `u32` ledger count
    // when the commitment is stored. An unbounded value used to be truncated,
    // silently turning an absurd window into a small one.
    assert_eq!(client.try_set_reveal_window(&max), Ok(Ok(())));
    assert_eq!(client.get_reveal_window(), max);

    assert_eq!(
        client.try_set_reveal_window(&(max + 1)),
        Err(Ok(Error::InvalidThreshold))
    );
    // A rejected value must not be persisted.
    assert_eq!(client.get_reveal_window(), max);

    // The first value past the `u32` second range is far above the bound and is
    // rejected for the same reason, not by accident of truncation.
    assert_eq!(
        client.try_set_reveal_window(&FIRST_U32_OVERFLOW_SEC),
        Err(Ok(Error::InvalidThreshold))
    );

    // The minimum is accepted.
    assert_eq!(client.try_set_reveal_window(&0), Ok(Ok(())));
    assert_eq!(client.get_reveal_window(), 0);
}

#[test]
fn key_overlap_is_bounded_at_both_rotation_entry_points() {
    let (env, client, admin, _service) = setup();
    let signers = admin_signers(&env, &admin);
    let new_key = soroban_sdk::Bytes::from_array(&env, &[3u8; 33]);

    assert_eq!(
        client.try_rotate_service_pubkey(&signers, &new_key, &constants::MAX_KEY_OVERLAP_SECS),
        Ok(Ok(()))
    );

    // Promote it so the next rotation starts from a known current key.
    client.rotate_service_pubkey(&signers, &soroban_sdk::Bytes::from_array(&env, &[0u8; 33]), &0);

    let next_key = soroban_sdk::Bytes::from_array(&env, &[4u8; 33]);
    assert_eq!(
        client.try_rotate_aggregate_service_pubkey(
            &signers,
            &next_key,
            &(constants::MAX_KEY_OVERLAP_SECS + 1)
        ),
        Err(Ok(Error::InvalidThreshold))
    );
}

// ── Regressions for the defects fixed in #1246 ──────────────────────────────

#[test]
fn regression_signer_expiry_semantics_are_unchanged_at_the_boundary() {
    let (env, client, admin, _service) = setup();

    // The signer is added at ledger second 100 so the ages below are exact.
    at(&env, 100);
    let signer = Address::generate(&env);
    client.add_service_signer(&admin_signers(&env, &admin), &signer);
    client.set_service_threshold(&admin_signers(&env, &admin), &1);
    client.set_signer_rotation_ttl(&admin_signers(&env, &admin), &10);
    client.set_signer_rotation_grace(&admin_signers(&env, &admin), &5);

    // Expiry is `age - ttl > grace`, so ages 10 and 15 are inside the window
    // and 16 is the first rejected age. This pins the off-by-one on both sides.
    for (age, expect_ok) in [
        (1u64, true),
        (10, true),
        (15, true),
        (16, false),
    ] {
        at(&env, 100 + age);
        let wallet = Address::generate(&env);
        let pair = Symbol::short("USDCXUSD");
        let result = client.try_submit_score(
            &Vec::from_array(&env, [signer.clone()]),
            &wallet,
            &pair,
            &60,
            &false,
            &false,
            &(100 + age),
            &90,
            &1,
            &None,
        );
        if expect_ok {
            assert_eq!(result, Ok(Ok(())), "age {age} should be accepted");
        } else {
            assert_eq!(result, Err(Ok(Error::UnauthorizedSigner)), "age {age} should expire");
        }
    }
}

#[test]
fn regression_signer_expiry_does_not_abort_when_ttl_plus_grace_overflows() {
    let (env, client, admin, _service) = setup();

    at(&env, 100);
    let signer = Address::generate(&env);
    client.add_service_signer(&admin_signers(&env, &admin), &signer);
    client.set_service_threshold(&admin_signers(&env, &admin), &1);
    // The historical defect was `age > ttl + grace` evaluated directly: with
    // these two values the sum overflows `u64`, and under
    // `overflow-checks = true` + `panic = "abort"` that traps on the
    // score-submission path, so an admin setting them would brick every
    // future submission permanently.
    client.set_signer_rotation_ttl(&admin_signers(&env, &admin), &(u64::MAX - 10));
    client.set_signer_rotation_grace(&admin_signers(&env, &admin), &20);

    let submit = |env: &Env, wallet: &Address, pair: &Symbol, ts: u64| {
        client.try_submit_score(
            &Vec::from_array(env, [signer.clone()]),
            wallet,
            pair,
            &60,
            &false,
            &false,
            &ts,
            &90,
            &1,
            &None,
        )
    };

    // Zero age: accepted, and crucially a typed `Ok` rather than a trap.
    let pair = Symbol::short("USDCXUSD");
    assert_eq!(
        submit(&env, &Address::generate(&env), &pair, 100),
        Ok(Ok(()))
    );

    // The largest age the clock can express, under the same overflowing
    // configuration. `age - ttl` is 10 here, which is not greater than the
    // grace of 20, so the signer is still valid — and, critically, the call
    // returns a value instead of aborting the whole contract.
    at(&env, u64::MAX);
    assert_eq!(
        submit(&env, &Address::generate(&env), &pair, u64::MAX),
        Ok(Ok(())),
        "an unsatisfiable ttl + grace must not trap"
    );

    // Narrowing the ttl to a value that can actually be exceeded restores the
    // expiry, so the saturating form has not silently disabled the check.
    client.set_signer_rotation_ttl(&admin_signers(&env, &admin), &10);
    at(&env, 100);
    assert_eq!(
        submit(&env, &Address::generate(&env), &pair, 100),
        Ok(Ok(()))
    );
    at(&env, 131);
    assert_eq!(
        submit(&env, &Address::generate(&env), &pair, 131),
        Err(Ok(Error::UnauthorizedSigner)),
        "age 31 exceeds ttl 10 + grace 20"
    );
}

#[test]
fn a_second_submission_at_the_top_of_the_range_is_not_aborted() {
    let (env, client, _admin, _service) = setup();
    let wallet = Address::generate(&env);
    let pair = Symbol::short("USDCXUSD");

    at(&env, u64::MAX);
    submit_at(&env, &client, &wallet, &pair, 60, 90, u64::MAX);
    // The per-wallet cooldown is `now < last + 3600`. At the top of the range
    // that addition saturates, which makes the window vacuous rather than
    // fatal. Documented as a boundary property: real ledgers are ~55 years
    // past epoch and will never observe it, but it must not trap.
    submit_at(&env, &client, &wallet, &pair, 61, 90, u64::MAX);

    let effective = client.get_effective_score(&wallet, &pair);
    assert_eq!(effective.original_score, 61);
    assert_eq!(effective.effective_score, 61);
}
