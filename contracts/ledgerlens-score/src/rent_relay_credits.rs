//! Second `#[contractimpl]` block for [`crate::LedgerLensScoreContract`].
//!
//! Groups four related-but-separable follow-up features, each tracked
//! against its own issue and its own commit:
//!
//! 1. Permissionless keeper TTL-extension rewards (`docs/rent-griefing-analysis.md`).
//! 2. The permissionless attested-relay path (`docs/replay-protection-audit.md`).
//! 3. Prepaid gate-query credits (`docs/security/`).
//! 4. The tiered gate fee schedule (`docs/operations/runbook.md`).
//!
//! Kept in its own file instead of appending to the already very large
//! `impl LedgerLensScoreContract` block in `lib.rs` — Rust allows multiple
//! inherent `impl` blocks for the same type in the same crate, and the
//! Soroban `#[contractimpl]` macro processes each block independently, so
//! this is equivalent to adding the functions inline but far easier to
//! review in isolation. Private helper functions defined here are visible
//! to `lib.rs`'s block (and vice versa) because both live under the crate
//! root's module tree.

use soroban_sdk::{contractimpl, token, Address, Env, Symbol, Vec};

use crate::errors::Error;
use crate::{events, storage, LedgerLensScoreContract};

#[contractimpl]
impl LedgerLensScoreContract {
    // ═════════════════════════════════════════════════════════════════════
    // Issue: Permissionless keeper TTL-extension reward
    // ═════════════════════════════════════════════════════════════════════
    //
    // See `docs/rent-griefing-analysis.md` for the threat model this closes:
    // liveness of score-entry TTLs currently depends on a privileged admin
    // calling `extend_entry_ttls`. This adds a permissionless keeper role,
    // paid a small governance-configured reward per entry actually renewed,
    // funded from a dedicated pool that only governance tops up — never from
    // user (gate-credit) balances, which live in an entirely separate
    // storage namespace and token (see the prepaid-credits section below).
    //
    // Anti-gaming, by construction rather than by bookkeeping:
    // - Eligibility (`storage::keeper_entry_eligible`) requires the entry's
    //   estimated remaining TTL to already be at/below the configured
    //   reward window — a keeper cannot be paid for renewing an entry that
    //   isn't actually close to expiring.
    // - Renewing an entry resets its estimated remaining TTL back to
    //   `SCORE_TTL_THRESHOLD`, far outside any sane reward window, so the
    //   *same* entry cannot be renewed-and-paid again until it has
    //   genuinely decayed back toward expiry — "once per entry per window"
    //   falls out of the existing touch/reindex mechanism for free, with no
    //   extra last-rewarded storage record needed.
    // - Splitting one entry's renewal across many small calls doesn't help:
    //   the first call already resets its TTL, so it is simply not
    //   eligible again in a later call within the same window.
    // - The batch cap (`KEEPER_BATCH_MAX`), the pool balance, and the
    //   optional per-ledger cap jointly bound the worst case payout of any
    //   single call and any single ledger.

    /// Governance: configures the SEP-41 token the keeper reward pool pays
    /// out in. Admin M-of-N.
    pub fn set_keeper_reward_token(
        env: Env,
        admin_signers: Vec<Address>,
        token: Address,
    ) -> Result<(), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        storage::set_keeper_reward_token(&env, &token);
        Ok(())
    }

    /// Governance top-up: transfers `amount` of the configured keeper
    /// reward token from the admin into the dedicated reward pool. Only the
    /// admin can fund the pool — user gate-credit deposits (a separate
    /// token/namespace) can never reach it.
    pub fn fund_keeper_reward_pool(
        env: Env,
        admin_signers: Vec<Address>,
        amount: i128,
    ) -> Result<i128, Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        if amount <= 0 {
            return Err(Error::InvalidScore);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        let admin = storage::get_admin(&env);
        let token = storage::get_keeper_reward_token(&env).ok_or(Error::KeeperRewardTokenNotSet)?;
        token::TokenClient::new(&env, &token).transfer(
            &admin,
            &env.current_contract_address(),
            &amount,
        );
        let new_balance = storage::credit_keeper_reward_pool(&env, amount);
        events::keeper_reward_pool_funded(&env, &admin, amount, new_balance);
        Ok(new_balance)
    }

    /// Governance: sets the per-entry reward, the reward-eligibility TTL
    /// window (ledgers), and the optional per-ledger payout cap (`0` = no
    /// cap beyond the pool balance itself).
    ///
    /// # Errors
    /// - [`Error::InvalidKeeperRewardParams`] if `reward_per_entry` exceeds
    ///   `MAX_KEEPER_REWARD_PER_ENTRY` or `window_ledgers` exceeds
    ///   `SCORE_TTL_THRESHOLD` (a wider window than that is meaningless —
    ///   the entry's own TTL estimate never exceeds the threshold).
    pub fn set_keeper_reward_params(
        env: Env,
        admin_signers: Vec<Address>,
        reward_per_entry: i128,
        window_ledgers: u32,
        per_ledger_cap: i128,
    ) -> Result<(), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        if !(0..=crate::constants::MAX_KEEPER_REWARD_PER_ENTRY).contains(&reward_per_entry)
            || window_ledgers > crate::constants::SCORE_TTL_THRESHOLD
            || per_ledger_cap < 0
        {
            return Err(Error::InvalidKeeperRewardParams);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        storage::set_keeper_reward_params(&env, reward_per_entry, window_ledgers, per_ledger_cap);
        events::keeper_reward_params_set(&env, reward_per_entry, window_ledgers, per_ledger_cap);
        Ok(())
    }

    pub fn get_keeper_reward_pool(env: Env) -> i128 {
        storage::get_keeper_reward_pool(&env)
    }

    /// Returns `(reward_per_entry, window_ledgers, per_ledger_cap)`.
    pub fn get_keeper_reward_params(env: Env) -> (i128, u32, i128) {
        (
            storage::get_keeper_reward_per_entry(&env),
            storage::get_keeper_reward_window(&env),
            storage::get_keeper_reward_per_ledger_cap(&env),
        )
    }

    /// Permissionless keeper entry point. Anyone may call this — no admin
    /// or service authorization is checked on the *entries*; `keeper` only
    /// authorizes itself as the reward-payout destination. Renews every
    /// eligible `(wallet, asset_pair)` entry in `entries` exactly as
    /// `extend_entry_ttls` does, then pays `keeper` a bounded reward for
    /// each one actually renewed, subject to the pool balance and the
    /// per-ledger cap.
    ///
    /// Ineligible or already-live-for-long entries are silently skipped
    /// (not renewed, not paid, not an error) rather than failing the whole
    /// batch — feed this the output of `get_expiring_entries` for the
    /// common case where every entry is eligible; see the `recovery` CLI's
    /// `keeper` subcommand for building well-prioritized batches from
    /// larger candidate sets.
    ///
    /// Returns `(entries_renewed, reward_paid)`. `reward_paid` can be less
    /// than `entries_renewed * reward_per_entry` if the pool or per-ledger
    /// cap is exhausted partway through the batch — renewal (liveness)
    /// still happens for the remaining eligible entries even once rewards
    /// run out, since that is the primary goal and costs the keeper nothing
    /// extra.
    ///
    /// # Errors
    /// - [`Error::NotInitialized`] if the contract has no admin yet.
    /// - [`Error::BatchTooLarge`] if `entries.len() > KEEPER_BATCH_MAX`.
    pub fn keeper_extend_entry_ttls(
        env: Env,
        keeper: Address,
        entries: Vec<(Address, Symbol)>,
    ) -> Result<(u32, i128), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        if entries.len() > crate::constants::KEEPER_BATCH_MAX {
            return Err(Error::BatchTooLarge);
        }
        // Only authorizes the reward payout destination — does not gate
        // which entries are eligible, keeping this genuinely permissionless.
        keeper.require_auth();

        let reward_per_entry = storage::get_keeper_reward_per_entry(&env);
        let mut renewed: u32 = 0;
        let mut reward_requested: i128 = 0;

        for i in 0..entries.len() {
            let (wallet, asset_pair) = entries.get(i).unwrap();
            if !storage::keeper_entry_eligible(&env, &wallet, &asset_pair) {
                continue;
            }
            if !storage::extend_score_entry_ttl(&env, &wallet, &asset_pair) {
                continue;
            }
            renewed += 1;
            if reward_per_entry > 0 {
                reward_requested = reward_requested.saturating_add(reward_per_entry);
            }
        }

        let mut reward_paid: i128 = 0;
        if reward_requested > 0 {
            reward_paid = storage::keeper_reward_payable(&env, reward_requested);
            if reward_paid > 0 {
                storage::debit_keeper_reward_pool(&env, reward_paid)?;
                storage::record_keeper_reward_spent(&env, reward_paid);
                let token =
                    storage::get_keeper_reward_token(&env).ok_or(Error::KeeperRewardTokenNotSet)?;
                token::TokenClient::new(&env, &token).transfer(
                    &env.current_contract_address(),
                    &keeper,
                    &reward_paid,
                );
            }
        }

        events::keeper_reward_paid(&env, &keeper, renewed, reward_paid);
        Ok((renewed, reward_paid))
    }
}
