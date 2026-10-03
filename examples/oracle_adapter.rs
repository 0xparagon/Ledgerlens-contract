//! Reference integration: registering a price-feed oracle with LedgerLens and
//! reading oracle-adjusted effective risk scores.
//!
//! This example demonstrates:
//! 1. Deploying a minimal oracle contract that implements `get_price(asset_pair) -> i128`.
//! 2. Registering it with LedgerLens via `register_oracle`.
//! 3. Calling `get_effective_score` and observing that the returned
//!    `confidence_floor` is elevated when the oracle reports a high price,
//!    signalling increased market uncertainty to the caller.
//! 4. **Staleness handling (issue #429)**: if the oracle's last price update is
//!    older than the admin-configured staleness threshold
//!    (`set_oracle_staleness_threshold` / `get_oracle_staleness_threshold`,
//!    default: 3 600 s), `get_effective_score` automatically falls back to an
//!    unadjusted confidence floor of `0` and emits an `orc_stale` event.
//!    Consumers can poll `is_oracle_stale(asset_pair)` to detect this condition
//!    without having to call `get_effective_score`.
//!
//! Build it as part of the workspace:
//!
//! ```text
//! cargo build --example oracle_adapter -p ledgerlens-score
//! ```
//!
//! The key insight: `confidence_floor` in [`EffectiveRiskScore`] lets a
//! composable protocol know *at query time* that current market conditions
//! reduce confidence in the static stored score.  It is advisory — the stored
//! score and confidence are never mutated.
//!
//! # Trust boundary (issue #1185)
//!
//! The oracle is an *external* contract chosen by governance. Everything it
//! returns crosses a trust boundary and MUST be treated as untrusted data.
//! The registry therefore never trusts the oracle to behave: it uses
//! non-panicking call forms, validates every return value explicitly, and
//! bounds the work a hostile oracle can force.
//!
//! ## External calls and the assumptions made about their results
//!
//! | Call site | Assumed result | Validation | Fallback |
//! |-----------|----------------|------------|----------|
//! | `get_price(pair) -> i128` | in-range `i128`, non-negative, fresh | `try_get_price` (non-panicking); reject negative / out-of-range; staleness check | floor `0`, emit `orc_stale` / `orc_invalid` |
//! | `get_price` return *type* | exactly `i128` | `try_get_price` returns `Err` on wrong type / trap | floor `0`, emit `orc_invalid` |
//! | `get_price` *size* | scalar, no collection | scalar ABI — no unbounded collection accepted | n/a |
//! | `get_price` *latency* | bounded | caller-supplied budget; no unbounded loop over oracle output | floor `0` |
//!
//! ## Bounded work
//!
//! The adapter performs exactly **one** cross-contract call per query and
//! writes **zero** ledger entries on behalf of the oracle. A hostile oracle
//! cannot force the registry to iterate a returned collection or to persist
//! attacker-controlled data: the only value consumed is a single `i128` that
//! is range-checked before use.

#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

/// Storage key used by the example oracle.
#[contracttype]
pub enum OracleKey {
    Price(Symbol),
}

/// Minimal price-feed oracle.
///
/// A production oracle would pull prices from an off-chain feed signed by a
/// trusted key. This example stores a price that the admin pushes on-chain,
/// sufficient to demonstrate the LedgerLens integration.
///
/// **The only requirement** LedgerLens places on a registered oracle is that
/// it exposes a `get_price(asset_pair: Symbol) -> i128` function callable via
/// cross-contract `invoke_contract`. This contract satisfies that interface.
#[contract]
pub struct ExamplePriceFeedOracle;

#[contractimpl]
impl ExamplePriceFeedOracle {
    /// Admin sets the latest price for an asset pair (production: feed via
    /// authenticated off-chain relayer).
    pub fn set_price(env: Env, asset_pair: Symbol, price: i128) {
        env.storage().instance().set(&OracleKey::Price(asset_pair), &price);
    }

    /// Returns the stored price for `asset_pair`, or `0` if none has been set.
    ///
    /// This is the function LedgerLens calls via cross-contract invocation
    /// when computing the oracle-adjusted `confidence_floor` in
    /// `get_effective_score`.
    pub fn get_price(env: Env, asset_pair: Symbol) -> i128 {
        env.storage()
            .instance()
            .get(&OracleKey::Price(asset_pair))
            .unwrap_or(0i128)
    }
}

// ── Hostile-oracle mock (issue #1185) ────────────────────────────────────────
//
// Used by tests and fuzzing to prove that every hostile behaviour produces a
// documented, deterministic outcome without corrupting registry state. Each
// mode is selected by the admin so a single deployed mock can exercise all
// cases. The registry must never panic, never persist oracle-controlled data,
// and always fall back to an unadjusted confidence floor of `0`.

/// Hostile behaviours the mock can be configured to exhibit.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HostileMode {
    /// Returns a value outside the accepted range (e.g. `i128::MAX`).
    OutOfRange,
    /// Traps / panics on call, simulating a malicious or buggy oracle.
    Panicking,
    /// Returns a negative price, which the registry must reject.
    Negative,
    /// Burns the caller's budget before returning (slow oracle).
    Slow,
}

/// Storage key used by the hostile mock.
#[contracttype]
pub enum HostileKey {
    Mode,
}

/// A deliberately hostile oracle used only in tests/fuzzing.
#[contract]
pub struct HostileOracle;

#[contractimpl]
impl HostileOracle {
    /// Configure which hostile behaviour `get_price` should exhibit.
    pub fn set_mode(env: Env, mode: HostileMode) {
        env.storage().instance().set(&HostileKey::Mode, &mode);
    }

    /// Hostile `get_price`. The registry calls this via a non-panicking form
    /// (`try_get_price`), so a trap here is caught and mapped to the
    /// documented fallback rather than aborting the registry call.
    pub fn get_price(env: Env, _asset_pair: Symbol) -> i128 {
        let mode: HostileMode = env
            .storage()
            .instance()
            .get(&HostileKey::Mode)
            .unwrap_or(HostileMode::OutOfRange);

        match mode {
            // Out-of-range: registry must reject and fall back to floor 0.
            HostileMode::OutOfRange => i128::MAX,
            // Negative: registry must reject and fall back to floor 0.
            HostileMode::Negative => -1i128,
            // Panicking: registry's non-panicking call form must catch this.
            HostileMode::Panicking => panic!("hostile oracle: intentional trap"),
            // Slow: burn budget, then return a plausible value. The registry
            // bounds work to a single call, so this cannot loop the caller.
            HostileMode::Slow => {
                let mut acc: i128 = 0;
                for i in 0..1_000i128 {
                    acc = acc.wrapping_add(i);
                }
                acc
            }
        }
    }
}

// ── Integration sketch (pseudo-code, requires a test environment) ─────────────
//
// 1. Deploy LedgerLens and the oracle:
//
//    let ll_id  = env.register_contract(None, LedgerLensScoreContract);
//    let orc_id = env.register_contract(None, ExamplePriceFeedOracle);
//    let ll  = LedgerLensScoreContractClient::new(&env, &ll_id);
//    let orc = ExamplePriceFeedOracleClient::new(&env, &orc_id);
//
// 2. Push a price into the oracle:
//
//    orc.set_price(&symbol_short!("XLM_USDC"), &500_000i128);
//
// 3. Register the oracle with LedgerLens (admin call):
//
//    ll.register_oracle(&admin_signers, &symbol_short!("XLM_USDC"), &orc_id);
//
// 4. Submit a risk score for a wallet:
//
//    ll.submit_score(&signers, &wallet, &symbol_short!("XLM_USDC"),
//                    &55, &false, &false, &ts, &90, &1, &None);
//
// 5. Query the effective score — confidence_floor will reflect the oracle price:
//
//    let eff = ll.get_effective_score(&wallet, &symbol_short!("XLM_USDC")).unwrap();
//    // price=500_000 → confidence_floor = 500_000 / 20_000 = 25
//    assert_eq!(eff.confidence_floor, 25);
//    assert_eq!(eff.original_score, 55);   // stored score unchanged
//    assert_eq!(eff.original_confidence, 90); // stored confidence unchanged
//
// 6. Callers should treat the effective confidence as:
//    effective_confidence = original_confidence.saturating_sub(confidence_floor)
//    A composable protocol can refuse a swap/borrow if effective_confidence
//    falls below its required minimum.
//
// 7. Hostile-oracle tests (issue #1185): register `HostileOracle` and assert
//    the documented fallback for each `HostileMode`:
//
//    let hostile_id = env.register_contract(None, HostileOracle);
//    let hostile = HostileOracleClient::new(&env, &hostile_id);
//    ll.register_oracle(&admin_signers, &symbol_short!("XLM_USDC"), &hostile_id);
//
//    for mode in [HostileMode::OutOfRange, HostileMode::Negative,
//                 HostileMode::Panicking, HostileMode::Slow] {
//        hostile.set_mode(&mode);
//        let eff = ll.get_effective_score(&wallet, &symbol_short!("XLM_USDC")).unwrap();
//        // Every hostile mode → unadjusted floor, stored state untouched.
//        assert_eq!(eff.confidence_floor, 0);
//        assert_eq!(eff.original_score, 55);
//        assert_eq!(eff.original_confidence, 90);
//    }
