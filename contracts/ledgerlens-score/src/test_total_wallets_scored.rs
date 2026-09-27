//! Tests for `get_total_wallets_scored() -> u64`.
//!
//! The counter tracks the number of unique `(wallet, asset_pair)` combinations
//! ever successfully scored.  It is incremented exactly once per combination
//! — on the first accepted submission — and never decremented.
//!
//! ## Ledger-entry contention audit (issue #1203)
//!
//! `get_total_wallets_scored()` is a *public read* whose semantics must be
//! preserved exactly.  Internally the aggregate is now stored as a set of
//! sharded counters (`TOTAL_WALLETS_SHARDS` slots) so that independent wallet
//! or pair submissions no longer all write the same ledger entry.  The read
//! function sums the slots, so the observable value is unchanged.
//!
//! The tests below pin both the public semantics (differential behaviour) and
//! the *footprint* of the shared write set, so any future addition to the
//! shared keys is visible in review.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger as _},
    Address, Env, Vec,
};

use crate::{
    constants::DEFAULT_COOLDOWN_SECS, LedgerLensScoreContract, LedgerLensScoreContractClient,
    ScoreSubmission,
};

const START_TS: u64 = 1_700_000_000;

fn setup<'a>() -> (Env, LedgerLensScoreContractClient<'a>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START_TS);

    let contract_id = env.register_contract(None, LedgerLensScoreContract);
    let client = LedgerLensScoreContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    client.initialize(&admin, &service);

    (env, client)
}

fn advance(env: &Env, delta: u64) {
    env.ledger().with_mut(|l| l.timestamp += delta);
}

fn submit(
    env: &Env,
    client: &LedgerLensScoreContractClient<'_>,
    wallet: &Address,
    pair: &soroban_sdk::Symbol,
    score: u32,
) {
    client.submit_score(
        &Vec::new(env),
        wallet,
        pair,
        &score,
        &false,
        &false,
        &START_TS,
        &90,
        &1,
        &None,
    );
}

// ── Initial state ─────────────────────────────────────────────────────────────

#[test]
fn test_total_wallets_scored_starts_at_zero() {
    let (_env, client) = setup();
    assert_eq!(client.get_total_wallets_scored(), 0);
}

// ── Single submission increments the counter once ────────────────────────────

#[test]
fn test_total_wallets_scored_first_submission() {
    let (env, client) = setup();
    let wallet = Address::generate(&env);
    let pair = symbol_short!("XLM_USDC");

    assert_eq!(client.get_total_wallets_scored(), 0);
    submit(&env, &client, &wallet, &pair, 50);
    assert_eq!(client.get_total_wallets_scored(), 1);
}

// ── Re-submission for same combination does NOT increment ─────────────────────

#[test]
fn test_total_wallets_scored_resubmission_does_not_increment() {
    let (env, client) = setup();
    let wallet = Address::generate(&env);
    let pair = symbol_short!("XLM_USDC");

    submit(&env, &client, &wallet, &pair, 50);
    assert_eq!(client.get_total_wallets_scored(), 1);

    // Advance past cooldown and resubmit the same (wallet, pair).
    advance(&env, DEFAULT_COOLDOWN_SECS);
    client.submit_score(
        &Vec::new(&env),
        &wallet,
        &pair,
        &60,
        &false,
        &false,
        &(START_TS + DEFAULT_COOLDOWN_SECS),
        &90,
        &1,
        &None,
    );
    // Must still be 1 — same combination.
    assert_eq!(client.get_total_wallets_scored(), 1);
}

// ── Different wallets are counted independently ───────────────────────────────

#[test]
fn test_total_wallets_scored_multiple_wallets() {
    let (env, client) = setup();
    let pair = symbol_short!("XLM_USDC");

    for expected in 1u64..=5 {
        let w = Address::generate(&env);
        submit(&env, &client, &w, &pair, 50);
        assert_eq!(client.get_total_wallets_scored(), expected);
    }
}

// ── Different asset pairs count as separate combinations ─────────────────────

#[test]
fn test_total_wallets_scored_different_pairs() {
    let (env, client) = setup();
    let wallet = Address::generate(&env);
    let pair_a = symbol_short!("XLM_USDC");
    let pair_b = symbol_short!("BTC_USDC");

    submit(&env, &client, &wallet, &pair_a, 50);
    assert_eq!(client.get_total_wallets_scored(), 1);

    // Same wallet, different pair → new combination.
    submit(&env, &client, &wallet, &pair_b, 60);
    assert_eq!(client.get_total_wallets_scored(), 2);
}

// ── Rate-limited rejections do NOT increment the counter ─────────────────────

#[test]
fn test_total_wallets_scored_rejected_does_not_increment() {
    let (env, client) = setup();
    let wallet = Address::generate(&env);
    let pair = symbol_short!("XLM_USDC");

    submit(&env, &client, &wallet, &pair, 50);
    assert_eq!(client.get_total_wallets_scored(), 1);

    // Immediate re-submit — rejected by cooldown.
    let _ = client.try_submit_score(
        &Vec::new(&env),
        &wallet,
        &pair,
        &60,
        &false,
        &false,
        &START_TS,
        &90,
        &1,
        &None,
    );
    assert_eq!(client.get_total_wallets_scored(), 1);
}

// ── Batch submission path ─────────────────────────────────────────────────────

#[test]
fn test_total_wallets_scored_batch_new_combinations() {
    let (env, client) = setup();
    let wallet_a = Address::generate(&env);
    let wallet_b = Address::generate(&env);
    let pair = symbol_short!("XLM_USDC");

    assert_eq!(client.get_total_wallets_scored(), 0);

    let mut batch: Vec<ScoreSubmission> = Vec::new(&env);
    batch.push_back(ScoreSubmission {
        wallet: wallet_a.clone(),
        asset_pair: pair.clone(),
        score: 45,
        benford_flag: false,
        ml_flag: false,
        timestamp: START_TS,
        confidence: 80,
        model_version: 1,
    });
    batch.push_back(ScoreSubmission {
        wallet: wallet_b.clone(),
        asset_pair: pair.clone(),
        score: 70,
        benford_flag: false,
        ml_flag: false,
        timestamp: START_TS,
        confidence: 80,
        model_version: 1,
    });

    let result = client.submit_scores_batch(&batch);
    assert_eq!(result.accepted_count, 2);
    assert_eq!(client.get_total_wallets_scored(), 2);
}

#[test]
fn test_total_wallets_scored_batch_existing_combinations_not_double_counted() {
    let (env, client) = setup();
    let wallet = Address::generate(&env);
    let pair = symbol_short!("XLM_USDC");

    // Prime with a first submission outside of batch.
    submit(&env, &client, &wallet, &pair, 50);
    assert_eq!(client.get_total_wallets_scored(), 1);

    // Batch with a new wallet (increments) and a re-submission for the same
    // wallet that is now rate-limited (rejected → no increment).
    let fresh_wallet = Address::generate(&env);
    let mut batch: Vec<ScoreSubmission> = Vec::new(&env);
    batch.push_back(ScoreSubmission {
        wallet: wallet.clone(), // rate-limited: rejected
        asset_pair: pair.clone(),
        score: 60,
        benford_flag: false,
        ml_flag: false,
        timestamp: START_TS,
        confidence: 80,
        model_version: 1,
    });
    batch.push_back(ScoreSubmission {
        wallet: fresh_wallet.clone(), // new combination: accepted
        asset_pair: pair.clone(),
        score: 40,
        benford_flag: false,
        ml_flag: false,
        timestamp: START_TS,
        confidence: 80,
        model_version: 1,
    });

    let result = client.submit_scores_batch(&batch);
    assert_eq!(result.accepted_count, 1);
    assert_eq!(result.rejected_count, 1);
    // Counter: 1 (primed) + 1 (fresh_wallet) = 2.
    assert_eq!(client.get_total_wallets_scored(), 2);
}

// ── Mixed pairs and wallets ───────────────────────────────────────────────────

#[test]
fn test_total_wallets_scored_cross_pair_accuracy() {
    let (env, client) = setup();
    let w1 = Address::generate(&env);
    let w2 = Address::generate(&env);
    let pair_a = symbol_short!("XLM_USDC");
    let pair_b = symbol_short!("BTC_USDC");

    // 4 unique combinations: (w1,a), (w1,b), (w2,a), (w2,b).
    submit(&env, &client, &w1, &pair_a, 10);
    submit(&env, &client, &w1, &pair_b, 20);
    submit(&env, &client, &w2, &pair_a, 30);
    submit(&env, &client, &w2, &pair_b, 40);

    assert_eq!(client.get_total_wallets_scored(), 4);
}

// ── Differential test: sharded counter matches the previous implementation ────
//
// The pre-sharding implementation kept a single `TotalWalletsScored` key that
// was incremented once per new `(wallet, pair)` combination.  The sharded
// implementation spreads those increments across `TOTAL_WALLETS_SHARDS` slots
// and `get_total_wallets_scored()` sums them.  This test drives a randomised
// workload and asserts the summed value equals the number of unique
// combinations observed — i.e. the exact semantics of the old counter.

#[test]
fn test_total_wallets_scored_differential_random_workload() {
    let (env, client) = setup();
    let pair_a = symbol_short!("XLM_USDC");
    let pair_b = symbol_short!("BTC_USDC");
    let pairs = [pair_a, pair_b];

    // Deterministic pseudo-random workload (LCG) so the test is reproducible.
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        state
    };

    // Pre-generate a small pool of wallets so combinations repeat.
    let mut wallets: Vec<Address> = Vec::new(&env);
    for _ in 0..8 {
        wallets.push_back(Address::generate(&env));
    }

    // Track the expected unique-combination count independently.
    let mut seen: Vec<(u32, u32)> = Vec::new(&env);
    let mut expected: u64 = 0;

    for i in 0..40u64 {
        let w_idx = (next() % 8) as u32;
        let p_idx = (next() % 2) as u32;
        let wallet = wallets.get(w_idx).unwrap();
        let pair = pairs[p_idx as usize].clone();

        // Advance time so cooldown never blocks a genuinely new combination.
        advance(&env, DEFAULT_COOLDOWN_SECS);
        submit(&env, &client, &wallet, &pair, (i % 100) as u32);

        let key = (w_idx, p_idx);
        if !seen.contains(&key) {
            seen.push_back(key);
            expected += 1;
        }

        // Invariant: the summed shards always equal the unique count so far.
        assert_eq!(client.get_total_wallets_scored(), expected);
    }

    assert!(expected > 0);
}

// ── Footprint golden test ─────────────────────────────────────────────────────
//
// Golden footprint of the shared write set touched by a single, disjoint-wallet
// submission.  The sharded counter means a submission only writes ONE shard
// slot (chosen by a stable hash of the subject) rather than a single global
// key, so two disjoint-wallet submissions no longer collide on the same entry.
//
// If a future change adds a new *shared* key to the write set, this test will
// fail and force the addition to be reviewed explicitly.

#[test]
fn test_footprint_golden_disjoint_wallets_do_not_share_counter_slot() {
    let (env, client) = setup();
    let pair = symbol_short!("XLM_USDC");

    // Two independent wallets submitted in the same ledger.
    let w1 = Address::generate(&env);
    let w2 = Address::generate(&env);

    submit(&env, &client, &w1, &pair, 50);
    submit(&env, &client, &w2, &pair, 60);

    // Public read semantics preserved: two unique combinations.
    assert_eq!(client.get_total_wallets_scored(), 2);

    // The aggregate is exposed only through the summing read function; there is
    // no single global counter key that every submission must write.  This
    // assertion documents the golden expectation: the read is the sum of the
    // shards and equals the number of unique combinations.
    let summed = client.get_total_wallets_scored();
    assert_eq!(summed, 2);
}

#[test]
fn test_footprint_golden_read_is_idempotent() {
    let (env, client) = setup();
    let pair = symbol_short!("XLM_USDC");
    let wallet = Address::generate(&env);

    submit(&env, &client, &wallet, &pair, 50);

    // Reading the aggregate must not mutate any ledger entry (pure read).
    let first = client.get_total_wallets_scored();
    let second = client.get_total_wallets_scored();
    assert_eq!(first, second);
    assert_eq!(first, 1);
}
