#![cfg(test)]
//! Unit tests for the permissionless keeper TTL-extension reward
//! (see `docs/rent-griefing-analysis.md` and `rent_relay_credits.rs`).

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
    Address, Env, Vec,
};

use crate::constants::SCORE_TTL_THRESHOLD;
use crate::{Error, LedgerLensScoreContract, LedgerLensScoreContractClient};

const START_SEQ: u32 = 1_000;

struct Setup<'a> {
    env: Env,
    client: LedgerLensScoreContractClient<'a>,
    token: Address,
}

fn setup(pool_funding: i128) -> Setup<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_sequence_number(START_SEQ);

    let contract = env.register_contract(None, LedgerLensScoreContract);
    let client = LedgerLensScoreContractClient::new(&env, &contract);
    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    client.initialize(&admin, &service);

    env.as_contract(&contract, || {
        env.storage().instance().extend_ttl(1_000_000, 6_000_000);
    });

    let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = sac.address();
    StellarAssetClient::new(&env, &token).mint(&admin, &1_000_000_000);
    client.set_keeper_reward_token(&Vec::new(&env), &token);
    if pool_funding > 0 {
        client.fund_keeper_reward_pool(&Vec::new(&env), &pool_funding);
    }

    let client: LedgerLensScoreContractClient<'static> = unsafe { core::mem::transmute(client) };
    Setup { env, client, token }
}

fn submit(s: &Setup, wallet: &Address, pair: soroban_sdk::Symbol) {
    s.client.submit_score(
        &Vec::new(&s.env),
        wallet,
        &pair,
        &10,
        &false,
        &false,
        &1,
        &90,
        &1,
        &None,
    );
}

// ── eligibility ──────────────────────────────────────────────────────────────

#[test]
fn ineligible_entry_too_early_is_skipped_not_rewarded() {
    let s = setup(10_000);
    s.client.set_keeper_reward_params(&Vec::new(&s.env), &100, &0, &0);
    let wallet = Address::generate(&s.env);
    let pair = symbol_short!("XLM_USDC");
    submit(&s, &wallet, pair.clone());

    let keeper = Address::generate(&s.env);
    let mut entries = Vec::new(&s.env);
    entries.push_back((wallet, pair));
    // Fresh submission — nowhere near the reward window (default window 0
    // means "already due"), so it must not be renewed or rewarded.
    let (renewed, paid) = s.client.keeper_extend_entry_ttls(&keeper, &entries);
    assert_eq!(renewed, 0);
    assert_eq!(paid, 0);
}

#[test]
fn eligible_entry_is_renewed_and_rewarded() {
    let s = setup(10_000);
    let reward: i128 = 250;
    s.client.set_keeper_reward_params(&Vec::new(&s.env), &reward, &0, &0);
    let wallet = Address::generate(&s.env);
    let pair = symbol_short!("XLM_USDC");
    submit(&s, &wallet, pair.clone());

    s.env.ledger().set_sequence_number(START_SEQ + SCORE_TTL_THRESHOLD);

    let keeper = Address::generate(&s.env);
    let mut entries = Vec::new(&s.env);
    entries.push_back((wallet.clone(), pair.clone()));
    let (renewed, paid) = s.client.keeper_extend_entry_ttls(&keeper, &entries);
    assert_eq!(renewed, 1);
    assert_eq!(paid, reward);
    assert_eq!(s.client.get_entry_ttl(&wallet, &pair), SCORE_TTL_THRESHOLD);
    assert_eq!(s.client.get_keeper_reward_pool(), 10_000 - reward);
}

#[test]
fn repeat_call_within_same_window_is_not_rewarded_again() {
    let s = setup(10_000);
    let reward: i128 = 250;
    s.client.set_keeper_reward_params(&Vec::new(&s.env), &reward, &0, &0);
    let wallet = Address::generate(&s.env);
    let pair = symbol_short!("XLM_USDC");
    submit(&s, &wallet, pair.clone());
    s.env.ledger().set_sequence_number(START_SEQ + SCORE_TTL_THRESHOLD);

    let keeper = Address::generate(&s.env);
    let mut entries = Vec::new(&s.env);
    entries.push_back((wallet, pair));

    let (renewed1, paid1) = s.client.keeper_extend_entry_ttls(&keeper, &entries);
    assert_eq!((renewed1, paid1), (1, reward));

    // Immediately calling again with the same entry: renewing it just reset
    // its estimated remaining TTL back to SCORE_TTL_THRESHOLD, which is far
    // outside the (default zero) reward window, so this must be a no-op —
    // not a second payout for the same work.
    let (renewed2, paid2) = s.client.keeper_extend_entry_ttls(&keeper, &entries);
    assert_eq!((renewed2, paid2), (0, 0));
    assert_eq!(s.client.get_keeper_reward_pool(), 10_000 - reward);
}

// ── pool exhaustion ──────────────────────────────────────────────────────────

#[test]
fn pool_exhausted_still_renews_but_stops_paying() {
    let s = setup(300); // enough for exactly one 250-reward entry, not two
    let reward: i128 = 250;
    s.client.set_keeper_reward_params(&Vec::new(&s.env), &reward, &0, &0);

    let pair = symbol_short!("XLM_USDC");
    let wallet_a = Address::generate(&s.env);
    let wallet_b = Address::generate(&s.env);
    submit(&s, &wallet_a, pair.clone());
    submit(&s, &wallet_b, pair.clone());
    s.env.ledger().set_sequence_number(START_SEQ + SCORE_TTL_THRESHOLD);

    let keeper = Address::generate(&s.env);
    let mut entries = Vec::new(&s.env);
    entries.push_back((wallet_a.clone(), pair.clone()));
    entries.push_back((wallet_b.clone(), pair.clone()));

    let (renewed, paid) = s.client.keeper_extend_entry_ttls(&keeper, &entries);
    // Both entries were genuinely due, so both get renewed (liveness is
    // preserved) even though the pool can only cover one reward.
    assert_eq!(renewed, 2);
    assert_eq!(paid, 250);
    assert_eq!(s.client.get_keeper_reward_pool(), 50);
    assert_eq!(s.client.get_entry_ttl(&wallet_a, &pair), SCORE_TTL_THRESHOLD);
    assert_eq!(s.client.get_entry_ttl(&wallet_b, &pair), SCORE_TTL_THRESHOLD);
}

#[test]
fn per_ledger_cap_bounds_payout_even_with_ample_pool() {
    let s = setup(1_000_000);
    let reward: i128 = 100;
    let per_ledger_cap: i128 = 150;
    s.client.set_keeper_reward_params(&Vec::new(&s.env), &reward, &0, &per_ledger_cap);

    let pair = symbol_short!("XLM_USDC");
    let wallet_a = Address::generate(&s.env);
    let wallet_b = Address::generate(&s.env);
    submit(&s, &wallet_a, pair.clone());
    submit(&s, &wallet_b, pair.clone());
    s.env.ledger().set_sequence_number(START_SEQ + SCORE_TTL_THRESHOLD);

    let keeper = Address::generate(&s.env);
    let mut entries = Vec::new(&s.env);
    entries.push_back((wallet_a, pair.clone()));
    entries.push_back((wallet_b, pair));

    let (renewed, paid) = s.client.keeper_extend_entry_ttls(&keeper, &entries);
    assert_eq!(renewed, 2);
    // Two entries want 200 total, but the per-ledger cap only allows 150.
    assert_eq!(paid, 150);
}

// ── batch bounds ─────────────────────────────────────────────────────────────

#[test]
fn oversized_batch_rejected() {
    let s = setup(0);
    let pair = symbol_short!("XLM_USDC");
    let mut entries = Vec::new(&s.env);
    for _ in 0..=crate::constants::KEEPER_BATCH_MAX {
        entries.push_back((Address::generate(&s.env), pair.clone()));
    }
    let keeper = Address::generate(&s.env);
    assert_eq!(
        s.client.try_keeper_extend_entry_ttls(&keeper, &entries),
        Err(Ok(Error::BatchTooLarge))
    );
}

#[test]
fn empty_batch_is_a_noop() {
    let s = setup(0);
    let keeper = Address::generate(&s.env);
    let (renewed, paid) = s.client.keeper_extend_entry_ttls(&keeper, &Vec::new(&s.env));
    assert_eq!((renewed, paid), (0, 0));
}

// ── zero reward configuration ─────────────────────────────────────────────────

#[test]
fn zero_reward_still_renews_without_any_transfer() {
    let s = setup(0);
    // No `set_keeper_reward_params` call — reward defaults to 0.
    let wallet = Address::generate(&s.env);
    let pair = symbol_short!("XLM_USDC");
    submit(&s, &wallet, pair.clone());
    s.env.ledger().set_sequence_number(START_SEQ + SCORE_TTL_THRESHOLD);

    let keeper = Address::generate(&s.env);
    let mut entries = Vec::new(&s.env);
    entries.push_back((wallet.clone(), pair.clone()));
    let (renewed, paid) = s.client.keeper_extend_entry_ttls(&keeper, &entries);
    assert_eq!(renewed, 1);
    assert_eq!(paid, 0);
    assert_eq!(s.client.get_entry_ttl(&wallet, &pair), SCORE_TTL_THRESHOLD);
}

// ── keepers cannot profit from entries that aren't actually close to expiry ──

#[test]
fn keeper_cannot_farm_reward_from_freshly_submitted_entries() {
    let s = setup(10_000);
    s.client.set_keeper_reward_params(&Vec::new(&s.env), &500, &0, &0);
    let pair = symbol_short!("XLM_USDC");
    let keeper = Address::generate(&s.env);
    let mut total_paid: i128 = 0;

    // A keeper repeatedly submits fresh wallets and immediately tries to
    // collect a reward for "renewing" them — none of these are due, so this
    // must never pay out, no matter how many times it's attempted.
    for _ in 0..5 {
        let wallet = Address::generate(&s.env);
        submit(&s, &wallet, pair.clone());
        let mut entries = Vec::new(&s.env);
        entries.push_back((wallet, pair.clone()));
        let (_, paid) = s.client.keeper_extend_entry_ttls(&keeper, &entries);
        total_paid += paid;
    }
    assert_eq!(total_paid, 0);
    assert_eq!(s.client.get_keeper_reward_pool(), 10_000);
}
