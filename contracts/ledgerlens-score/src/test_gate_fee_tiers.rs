#![cfg(test)]
//! Unit tests for the tiered gate fee schedule (`query_risk_gate_metered`).
//! See `docs/operations/runbook.md` ("Tiered gate query fees") and
//! `fee_schedule.rs` for the pure per-call fee computation, which has its
//! own dedicated unit tests colocated in that module.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
    Address, Env, Vec,
};

use crate::{Error, FeeTier, LedgerLensScoreContract, LedgerLensScoreContractClient};

struct Setup<'a> {
    env: Env,
    client: LedgerLensScoreContractClient<'a>,
}

fn setup() -> Setup<'static> {
    let env = Env::default();
    env.mock_all_auths();
    let contract = env.register_contract(None, LedgerLensScoreContract);
    let client = LedgerLensScoreContractClient::new(&env, &contract);
    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    client.initialize(&admin, &service);

    let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = sac.address();
    client.set_gate_credit_token(&Vec::new(&env), &token);

    let client: LedgerLensScoreContractClient<'static> = unsafe { core::mem::transmute(client) };
    Setup { env, client }
}

fn fund(s: &Setup, consumer: &Address, amount: i128) {
    let token = s.client.get_gate_credit_token().unwrap();
    StellarAssetClient::new(&s.env, &token).mint(consumer, &amount);
    s.client.deposit_gate_credits(consumer, &amount);
}

fn three_tiers(env: &Env) -> Vec<FeeTier> {
    Vec::from_array(
        env,
        [
            FeeTier { min_volume: 0, fee: 100 },
            FeeTier { min_volume: 3, fee: 50 },
            FeeTier { min_volume: 6, fee: 10 },
        ],
    )
}

// ── tier boundaries ──────────────────────────────────────────────────────────

#[test]
fn calls_are_priced_at_the_correct_tier_at_each_boundary() {
    let s = setup();
    s.client.set_fee_tier_schedule(&Vec::new(&s.env), &three_tiers(&s.env));
    let consumer = Address::generate(&s.env);
    fund(&s, &consumer, 10_000);

    let wallet = Address::generate(&s.env);
    let pair = symbol_short!("XLM_USDC");

    // Calls 1-3 (volume 0,1,2): tier 0 = 100.
    for _ in 0..3 {
        s.client.query_risk_gate_metered(&consumer, &wallet, &pair, &75);
    }
    // Calls 4-6 (volume 3,4,5): tier 1 = 50.
    for _ in 0..3 {
        s.client.query_risk_gate_metered(&consumer, &wallet, &pair, &75);
    }
    // Call 7 (volume 6): tier 2 = 10.
    s.client.query_risk_gate_metered(&consumer, &wallet, &pair, &75);

    let expected_spent = 100 * 3 + 50 * 3 + 10;
    assert_eq!(s.client.get_gate_credit_balance(&consumer), 10_000 - expected_spent);
    assert_eq!(s.client.get_gate_credit_revenue(), expected_spent);
}

// ── window rollover ──────────────────────────────────────────────────────────

#[test]
fn window_rollover_resets_volume_and_pricing() {
    let s = setup();
    s.client.set_fee_tier_schedule(&Vec::new(&s.env), &three_tiers(&s.env));
    s.client.set_fee_tier_window_ledgers(&Vec::new(&s.env), &10);
    let consumer = Address::generate(&s.env);
    fund(&s, &consumer, 10_000);
    let wallet = Address::generate(&s.env);
    let pair = symbol_short!("XLM_USDC");

    // Reach tier 1 (volume 3) within the first window.
    for _ in 0..3 {
        s.client.query_risk_gate_metered(&consumer, &wallet, &pair, &75);
    }
    let balance_after_first_window = s.client.get_gate_credit_balance(&consumer);
    // Spent 100*3 = 300 so far, all tier 0.
    assert_eq!(balance_after_first_window, 10_000 - 300);

    // Advance well past the window length.
    s.env.ledger().set_sequence_number(s.env.ledger().sequence() + 11);

    // Volume resets to 0 -- this call is priced at tier 0 (100) again, not
    // tier 1, even though the consumer's lifetime volume is already 3.
    s.client.query_risk_gate_metered(&consumer, &wallet, &pair, &75);
    assert_eq!(s.client.get_gate_credit_balance(&consumer), balance_after_first_window - 100);
}

// ── exemption expiry ─────────────────────────────────────────────────────────

#[test]
fn active_exemption_charges_zero_fee() {
    let s = setup();
    s.client.set_fee_tier_schedule(&Vec::new(&s.env), &three_tiers(&s.env));
    let consumer = Address::generate(&s.env);
    fund(&s, &consumer, 10_000);
    let wallet = Address::generate(&s.env);
    let pair = symbol_short!("XLM_USDC");

    let now = s.env.ledger().timestamp();
    s.client.set_fee_exemption(&Vec::new(&s.env), &consumer, &(now + 1_000), &1);

    s.client.query_risk_gate_metered(&consumer, &wallet, &pair, &75);
    assert_eq!(s.client.get_gate_credit_balance(&consumer), 10_000);
}

#[test]
fn expired_exemption_no_longer_applies() {
    let s = setup();
    s.client.set_fee_tier_schedule(&Vec::new(&s.env), &three_tiers(&s.env));
    let consumer = Address::generate(&s.env);
    fund(&s, &consumer, 10_000);
    let wallet = Address::generate(&s.env);
    let pair = symbol_short!("XLM_USDC");

    let now = s.env.ledger().timestamp();
    s.client.set_fee_exemption(&Vec::new(&s.env), &consumer, &(now + 100), &1);
    s.env.ledger().set_timestamp(now + 200);

    assert!(s.client.get_fee_exemption(&consumer).is_none());
    s.client.query_risk_gate_metered(&consumer, &wallet, &pair, &75);
    // Tier 0 fee (100) was charged -- exemption had already lapsed.
    assert_eq!(s.client.get_gate_credit_balance(&consumer), 10_000 - 100);
}

#[test]
fn set_fee_exemption_rejects_non_future_expiry() {
    let s = setup();
    let consumer = Address::generate(&s.env);
    let now = s.env.ledger().timestamp();
    assert_eq!(
        s.client.try_set_fee_exemption(&Vec::new(&s.env), &consumer, &now, &1),
        Err(Ok(Error::InvalidExemptionExpiry))
    );
}

// ── zero-fee case (no schedule configured) ────────────────────────────────────

#[test]
fn zero_fee_when_no_schedule_configured() {
    let s = setup(); // no set_fee_tier_schedule call -- defaults to empty
    let consumer = Address::generate(&s.env);
    // No deposit at all -- if this weren't zero-fee, the call would fail
    // with InsufficientGateCredits.
    let wallet = Address::generate(&s.env);
    let pair = symbol_short!("XLM_USDC");
    let result = s.client.query_risk_gate_metered(&consumer, &wallet, &pair, &75);
    assert!(!result); // no score submitted -> gate returns false, but the call itself succeeded
    assert_eq!(s.client.get_gate_credit_balance(&consumer), 0);
}

// ── insufficient credits ──────────────────────────────────────────────────────

#[test]
fn insufficient_credits_rejects_the_call() {
    let s = setup();
    s.client.set_fee_tier_schedule(&Vec::new(&s.env), &three_tiers(&s.env));
    let consumer = Address::generate(&s.env);
    fund(&s, &consumer, 50); // less than the 100-stroop tier-0 fee
    let wallet = Address::generate(&s.env);
    let pair = symbol_short!("XLM_USDC");

    assert_eq!(
        s.client.try_query_risk_gate_metered(&consumer, &wallet, &pair, &75),
        Err(Ok(Error::InsufficientGateCredits))
    );
    // Balance is untouched -- no partial debit on rejection.
    assert_eq!(s.client.get_gate_credit_balance(&consumer), 50);
}

// ── schedule validation ────────────────────────────────────────────────────────

#[test]
fn schedule_must_be_strictly_ascending_by_min_volume() {
    let s = setup();
    let bad = Vec::from_array(
        &s.env,
        [FeeTier { min_volume: 10, fee: 50 }, FeeTier { min_volume: 5, fee: 100 }],
    );
    assert_eq!(
        s.client.try_set_fee_tier_schedule(&Vec::new(&s.env), &bad),
        Err(Ok(Error::InvalidFeeTierSchedule))
    );
}

#[test]
fn schedule_rejects_negative_fee() {
    let s = setup();
    let bad = Vec::from_array(&s.env, [FeeTier { min_volume: 0, fee: -1 }]);
    assert_eq!(
        s.client.try_set_fee_tier_schedule(&Vec::new(&s.env), &bad),
        Err(Ok(Error::InvalidFeeTierSchedule))
    );
}

// ── property: total fees collected equals the sum of charged fees per call ───

#[test]
fn total_debited_equals_sum_of_per_call_fees() {
    let s = setup();
    s.client.set_fee_tier_schedule(&Vec::new(&s.env), &three_tiers(&s.env));
    let consumer = Address::generate(&s.env);
    fund(&s, &consumer, 100_000);
    let wallet = Address::generate(&s.env);
    let pair = symbol_short!("XLM_USDC");

    let starting_balance = s.client.get_gate_credit_balance(&consumer);
    let mut sum_of_charged_fees: i128 = 0;
    for i in 0u32..20 {
        let before = s.client.get_gate_credit_balance(&consumer);
        s.client.query_risk_gate_metered(&consumer, &wallet, &pair, &75);
        let after = s.client.get_gate_credit_balance(&consumer);
        let charged_this_call = before - after;
        sum_of_charged_fees += charged_this_call;
        // Cross-check against the pure function directly, keyed by the same
        // volume-before-call index used on-chain (0-indexed call count).
        let expected = crate::fee_schedule::compute_gate_fee(&three_tiers(&s.env), i);
        assert_eq!(charged_this_call, expected);
    }
    let ending_balance = s.client.get_gate_credit_balance(&consumer);
    assert_eq!(starting_balance - ending_balance, sum_of_charged_fees);
    assert_eq!(s.client.get_gate_credit_revenue(), sum_of_charged_fees);
}
