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

use soroban_sdk::{
    contractimpl, crypto::Hash, token, Address, Bytes, BytesN, Env, Symbol, SymbolStr,
    TryFromVal, Vec,
};
use subtle::ConstantTimeEq;

use crate::errors::Error;
use crate::fee_schedule::compute_gate_fee;
use crate::types::{FeeTier, RelayScoreAttestation};
use crate::{events, storage, LedgerLensScoreContract, LedgerLensScoreContractClient};

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

    // ═════════════════════════════════════════════════════════════════════
    // Issue: Permissionless attested relay
    // ═════════════════════════════════════════════════════════════════════
    //
    // See `docs/replay-protection-audit.md`. `submit_score`'s single-key
    // attestation path already lets the service prove a submission's
    // origin cryptographically, but it *additionally* requires the service
    // account itself to satisfy Soroban `require_auth()` on the submitting
    // transaction — so only the service account can actually be the
    // transaction submitter today. `relay_attested_score` removes that
    // requirement for a new, more tightly-bound attestation variant
    // (`RelayScoreAttestation`) that is the *sole* authorization for the
    // state change, so any relayer can submit it (enabling fee-sponsored
    // relaying / removing the "who's online to submit" single point of
    // failure) without weakening replay protection.
    //
    // Governance: configures the SEP-41 token and per-window cap the
    // optional relayer tip is paid from/bounded by.
    pub fn set_relay_tip_token(
        env: Env,
        admin_signers: Vec<Address>,
        token: Address,
    ) -> Result<(), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        storage::set_relay_tip_token(&env, &token);
        Ok(())
    }

    pub fn fund_relay_tip_pool(
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
        let token = storage::get_relay_tip_token(&env).ok_or(Error::RelayTipTokenNotSet)?;
        token::TokenClient::new(&env, &token).transfer(
            &admin,
            &env.current_contract_address(),
            &amount,
        );
        Ok(storage::credit_relay_tip_pool(&env, amount))
    }

    /// Governance: sets the flat tip paid to the relayer of the first
    /// accepted attestation, and the rolling per-window cap on total tips
    /// (anti-abuse: bounds how much a burst of relaying can drain the pool
    /// in a short span, independent of the per-attestation "only the first
    /// acceptance is ever tipped" rule below).
    pub fn set_relay_tip_params(
        env: Env,
        admin_signers: Vec<Address>,
        amount: i128,
        window_cap: i128,
        window_ledgers: u32,
    ) -> Result<(), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        if amount < 0 || window_cap < 0 {
            return Err(Error::InvalidKeeperRewardParams);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        storage::set_relay_tip_params(&env, amount, window_cap, window_ledgers);
        Ok(())
    }

    /// Permissionless relay of a service-signed score submission. Any
    /// relayer may call this — the attestation itself, not the caller's
    /// identity, is what authorizes the state change. `relayer` is only
    /// used as the destination for the optional tip payout.
    ///
    /// # Idempotency & ordering
    /// The attestation's recomputed commitment digest doubles as an
    /// idempotency key. If a second relayer posts the *same* attestation
    /// (same wallet/pair/score/flags/timestamp/confidence/model_version/
    /// nonce/contract binding/expiry — i.e. the same digest), this call is
    /// a cheap, well-defined no-op: a single storage read, then `Ok(false)`
    /// with no further state change and no signature verification, nonce
    /// mutation, or tip payout. The resulting on-chain state — the score
    /// value and the signer nonce — is therefore independent of which
    /// relayer's transaction happens to land first; only the first to land
    /// does any work, and every subsequent relay of the same attestation
    /// observes (not re-derives) that outcome.
    ///
    /// # Binding
    /// The commitment binds the score payload, `nonce` (so the same
    /// signature can't be replayed against a different nonce slot —
    /// `ScoreAttestation`'s legacy commitment does not include nonce, which
    /// this format fixes for the relay path), `valid_before_ledger` (a
    /// ledger-sequence freshness bound — a stale attestation past this
    /// ledger is rejected outright, so a leaked-but-expired attestation
    /// can't be relayed indefinitely), the contract address, the network
    /// passphrase, and the contract version. See
    /// `compute_relay_commitment`.
    ///
    /// # Errors
    /// - [`Error::NotInitialized`], [`Error::ContractPaused`],
    ///   [`Error::EpochClosed`] — same guards as `submit_score`.
    /// - [`Error::StaleAttestation`] — `valid_before_ledger` has passed, or
    ///   `contract_version` no longer matches.
    /// - [`Error::InvalidAttestation`] — signature or nonce mismatch.
    #[allow(clippy::too_many_arguments)]
    pub fn relay_attested_score(
        env: Env,
        relayer: Address,
        wallet: Address,
        asset_pair: Symbol,
        score: u32,
        benford_flag: bool,
        ml_flag: bool,
        timestamp: u64,
        confidence: u32,
        model_version: u32,
        attestation: RelayScoreAttestation,
    ) -> Result<bool, Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        if storage::is_frozen(&env) {
            return Err(Error::ContractPaused);
        }
        Self::ensure_asset_pair_bounded(&env, &asset_pair)?;
        if storage::is_paused(&env) {
            return Err(Error::ContractPaused);
        }
        if storage::is_pair_paused(&env, &asset_pair) {
            return Err(Error::ContractPaused);
        }
        if !storage::is_epoch_open(&env) {
            return Err(Error::EpochClosed);
        }
        // Only authorizes the tip-payout destination; the attestation is
        // the actual authorization for the submission itself.
        relayer.require_auth();

        if attestation.contract_version != storage::get_contract_version(&env) {
            return Err(Error::StaleAttestation);
        }
        if env.ledger().sequence() > attestation.valid_before_ledger {
            return Err(Error::StaleAttestation);
        }

        let digest = Self::compute_relay_commitment(
            &env,
            &wallet,
            &asset_pair,
            score,
            benford_flag,
            ml_flag,
            timestamp,
            confidence,
            model_version,
            &attestation,
        )?;
        if digest.to_array().ct_eq(&attestation.commitment.to_array()).unwrap_u8() == 0 {
            return Err(Error::InvalidAttestation);
        }

        let digest_key = BytesN::<32>::from_array(&env, &digest.to_array());
        if storage::is_relayed_attestation_used(&env, &digest_key) {
            events::relay_duplicate_noop(&env, &relayer);
            return Ok(false);
        }

        Self::verify_signature(&env, &digest, &attestation.signature)?;

        let service = storage::get_service(&env);
        let current_nonce = storage::get_signer_nonce(&env, &service);
        if current_nonce != attestation.nonce {
            return Err(Error::InvalidAttestation);
        }
        let next_nonce = attestation.nonce.checked_add(1).ok_or(Error::InvalidAttestation)?;

        // Mark used before finalizing so a reentrant/nested call (if the
        // host ever allowed one) can't double-apply the same attestation.
        storage::mark_relayed_attestation_used(&env, &digest_key);
        storage::set_signer_nonce(&env, &service, next_nonce);

        Self::finalize_score_submission(
            &env,
            &Vec::new(&env),
            &wallet,
            &asset_pair,
            score,
            benford_flag,
            ml_flag,
            timestamp,
            confidence,
            model_version,
            None,
        )?;
        events::relay_accepted(&env, &relayer, &wallet, &asset_pair);

        let tip = storage::get_relay_tip_amount(&env);
        if tip > 0 {
            if let Some(token) = storage::get_relay_tip_token(&env) {
                let payable = storage::relay_tip_payable(&env, tip);
                if payable > 0 {
                    storage::debit_relay_tip_pool(&env, payable)?;
                    token::TokenClient::new(&env, &token).transfer(
                        &env.current_contract_address(),
                        &relayer,
                        &payable,
                    );
                    events::relay_tip_paid(&env, &relayer, payable);
                }
            }
        }

        Ok(true)
    }
}

impl LedgerLensScoreContract {
    /// Builds the canonical relay-commitment preimage and hashes it with
    /// SHA-256. Mirrors `Self::compute_commitment` (the legacy
    /// `ScoreAttestation` preimage) field-for-field, but additionally binds
    /// `nonce` and `valid_before_ledger` — see `relay_attested_score`'s doc
    /// comment for why those two fields must be part of the signed digest
    /// for a permissionlessly-relayable attestation, where (unlike
    /// `submit_score`) there is no accompanying `require_auth` to fall back
    /// on.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compute_relay_commitment(
        env: &Env,
        wallet: &Address,
        asset_pair: &Symbol,
        score: u32,
        benford_flag: bool,
        ml_flag: bool,
        timestamp: u64,
        confidence: u32,
        model_version: u32,
        attestation: &RelayScoreAttestation,
    ) -> Result<Hash<32>, Error> {
        let pair_str = SymbolStr::try_from_val(env, &asset_pair.to_symbol_val())
            .map_err(|_| Error::InvalidAttestation)?;
        let pair_bytes: &[u8] = pair_str.as_ref();
        if pair_bytes.len() > 9 {
            return Err(Error::InvalidAttestation);
        }
        let mut pair_buf = [0u8; 9];
        pair_buf[..pair_bytes.len()].copy_from_slice(pair_bytes);

        let mut wallet_buf = [0u8; 56];
        wallet.to_string().copy_into_slice(&mut wallet_buf);

        let mut contract_buf = [0u8; 56];
        env.current_contract_address().to_string().copy_into_slice(&mut contract_buf);

        let mut preimage = Bytes::new(env);
        preimage.extend_from_array(&wallet_buf);
        preimage.extend_from_array(&pair_buf);
        preimage.extend_from_array(&score.to_le_bytes());
        preimage.push_back(benford_flag as u8);
        preimage.push_back(ml_flag as u8);
        preimage.extend_from_array(&timestamp.to_le_bytes());
        preimage.extend_from_array(&confidence.to_le_bytes());
        preimage.extend_from_array(&model_version.to_le_bytes());
        preimage.extend_from_array(&contract_buf);
        preimage.extend_from_array(&env.ledger().network_id().to_array());
        preimage.extend_from_array(&attestation.contract_id.to_array());
        preimage.extend_from_array(&attestation.contract_version.to_le_bytes());
        preimage.extend_from_array(&attestation.nonce.to_le_bytes());
        preimage.extend_from_array(&attestation.valid_before_ledger.to_le_bytes());

        Ok(env.crypto().sha256(&preimage))
    }
}

#[contractimpl]
impl LedgerLensScoreContract {
    // ═════════════════════════════════════════════════════════════════════
    // Issue: Prepaid gate-query credits
    // ═════════════════════════════════════════════════════════════════════
    //
    // See `docs/security/prepaid-gate-credits.md`. Consumers deposit once
    // into a per-depositor credit balance; metered gate queries
    // (`query_risk_gate_metered`, in the tiered-fee-schedule section below)
    // debit that balance instead of doing a token transfer on every call.
    // Token transfers happen *only* on deposit and withdraw — a debit is
    // pure accounting, moving the debited amount from the depositor's
    // liability bucket into governance-withdrawable revenue without moving
    // any tokens, which is what keeps `query_risk_gate_metered` cheap.
    //
    // Solvency: `GateCreditLiabilityTotal` (what's owed back to
    // depositors) and `GateCreditRevenue` (governance-withdrawable) are
    // both O(1) running counters, disjoint from each other and from every
    // other token balance the contract tracks (the keeper-reward pool and
    // relay-tip pool above, and the legacy `FeeToken`/`AccumulatedFees`
    // path are all separate storage keys and may even be separate tokens).
    // The contract's `GateCreditToken` holdings must always be >=
    // liability + revenue; see `test_gate_credits.rs` for the property-style
    // check across randomized operation sequences.
    //
    // Depositor protection: withdrawals are *never* gated by `is_paused`,
    // and there is no admin/governance operation anywhere in this contract
    // that can move funds out of `GateCreditLiabilityTotal` other than a
    // depositor's own authorized withdrawal — governance can configure
    // fees going forward and can withdraw already-earned `GateCreditRevenue`,
    // but can never seize unspent deposited credit.

    /// Governance: configures the SEP-41 token accepted for gate-credit
    /// deposits. Changing this does **not** retroactively convert existing
    /// balances — they remain denominated in whatever token was configured
    /// when deposited. Operators changing this should ensure outstanding
    /// balances in the old token are fully withdrawn first (`get_gate_credit_token`
    /// lets integrators verify which token is currently active before
    /// depositing).
    pub fn set_gate_credit_token(
        env: Env,
        admin_signers: Vec<Address>,
        token: Address,
    ) -> Result<(), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        storage::set_gate_credit_token(&env, &token);
        Ok(())
    }

    pub fn get_gate_credit_token(env: Env) -> Option<Address> {
        storage::get_gate_credit_token(&env)
    }

    /// Governance: sets the delay (seconds) a withdrawal request must wait
    /// before `withdraw_gate_credits` can complete it. `0` disables the
    /// delay (immediate withdrawal).
    pub fn set_gate_credit_withdrawal_delay(
        env: Env,
        admin_signers: Vec<Address>,
        delay_secs: u64,
    ) -> Result<(), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        storage::set_gate_credit_withdrawal_delay(&env, delay_secs);
        Ok(())
    }

    pub fn get_gate_credit_balance(env: Env, depositor: Address) -> i128 {
        storage::get_gate_credit_balance(&env, &depositor)
    }

    /// Deposits `amount` of the configured gate-credit token from
    /// `depositor` into their credit balance. The only token transfer in
    /// the deposit path.
    ///
    /// # Errors
    /// - [`Error::InvalidCreditAmount`] if `amount <= 0`.
    /// - [`Error::GateCreditTokenNotSet`] if no token is configured.
    pub fn deposit_gate_credits(env: Env, depositor: Address, amount: i128) -> Result<i128, Error> {
        if amount <= 0 {
            return Err(Error::InvalidCreditAmount);
        }
        depositor.require_auth();
        let token = storage::get_gate_credit_token(&env).ok_or(Error::GateCreditTokenNotSet)?;
        token::TokenClient::new(&env, &token).transfer(
            &depositor,
            &env.current_contract_address(),
            &amount,
        );
        let new_balance = storage::credit_gate_balance(&env, &depositor, amount);
        storage::add_gate_credit_liability(&env, amount);
        events::gate_credit_deposited(&env, &depositor, amount, new_balance);
        Ok(new_balance)
    }

    /// Requests withdrawal of `amount` from `depositor`'s balance. The
    /// amount is still counted in the depositor's spendable balance (and
    /// thus still debitable by a metered query) until
    /// `withdraw_gate_credits` actually completes it — this call only
    /// starts the optional timelock; it does not reserve funds.
    ///
    /// # Errors
    /// - [`Error::InvalidCreditAmount`] if `amount <= 0` or exceeds the
    ///   depositor's current balance.
    pub fn withdraw_gate_credits_request(
        env: Env,
        depositor: Address,
        amount: i128,
    ) -> Result<u64, Error> {
        depositor.require_auth();
        let balance = storage::get_gate_credit_balance(&env, &depositor);
        if amount <= 0 || amount > balance {
            return Err(Error::InvalidCreditAmount);
        }
        let delay = storage::get_gate_credit_withdrawal_delay(&env);
        let unlock_at = env.ledger().timestamp().saturating_add(delay);
        storage::set_gate_credit_withdrawal_request(&env, &depositor, amount, unlock_at);
        events::gate_credit_withdrawal_requested(&env, &depositor, amount, unlock_at);
        Ok(unlock_at)
    }

    /// Completes a previously-requested withdrawal once its unlock time has
    /// passed, transferring `amount` back to `depositor`. **Never** gated
    /// by `is_paused` or any other admin-controlled switch — unspent credit
    /// is always eventually withdrawable by its depositor, by design.
    ///
    /// # Errors
    /// - [`Error::NoWithdrawalRequest`] if no request is pending.
    /// - [`Error::WithdrawalNotYetUnlocked`] if called before the delay
    ///   from `withdraw_gate_credits_request` has elapsed.
    /// - [`Error::InvalidCreditAmount`] if the balance has since dropped
    ///   below the requested amount (e.g. spent on metered queries after
    ///   the request was made) — the request is cleared either way so the
    ///   depositor can request a smaller amount.
    pub fn withdraw_gate_credits(env: Env, depositor: Address) -> Result<i128, Error> {
        depositor.require_auth();
        let (amount, unlock_at) = storage::get_gate_credit_withdrawal_request(&env, &depositor)
            .ok_or(Error::NoWithdrawalRequest)?;
        if env.ledger().timestamp() < unlock_at {
            return Err(Error::WithdrawalNotYetUnlocked);
        }
        storage::clear_gate_credit_withdrawal_request(&env, &depositor);
        let balance = storage::get_gate_credit_balance(&env, &depositor);
        if amount > balance {
            return Err(Error::InvalidCreditAmount);
        }
        storage::debit_gate_balance(&env, &depositor, amount)?;
        storage::sub_gate_credit_liability(&env, amount);
        let token = storage::get_gate_credit_token(&env).ok_or(Error::GateCreditTokenNotSet)?;
        token::TokenClient::new(&env, &token).transfer(
            &env.current_contract_address(),
            &depositor,
            &amount,
        );
        events::gate_credit_withdrawn(&env, &depositor, amount);
        Ok(amount)
    }

    pub fn get_gate_credit_liability_total(env: Env) -> i128 {
        storage::get_gate_credit_liability_total(&env)
    }

    pub fn get_gate_credit_revenue(env: Env) -> i128 {
        storage::get_gate_credit_revenue(&env)
    }

    /// Governance withdrawal of already-debited fee revenue. Scoped
    /// entirely to `GateCreditRevenue` / `GateCreditToken` — structurally
    /// unable to touch `GateCreditLiabilityTotal`, so this can never seize
    /// depositor funds regardless of `amount`.
    pub fn withdraw_gate_credit_revenue(
        env: Env,
        admin_signers: Vec<Address>,
        recipient: Address,
        amount: i128,
    ) -> Result<(), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        if amount <= 0 {
            return Err(Error::InvalidCreditAmount);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        storage::sub_gate_credit_revenue(&env, amount)?;
        let token = storage::get_gate_credit_token(&env).ok_or(Error::GateCreditTokenNotSet)?;
        token::TokenClient::new(&env, &token).transfer(
            &env.current_contract_address(),
            &recipient,
            &amount,
        );
        events::gate_credit_revenue_withdrawn(&env, &recipient, amount);
        Ok(())
    }
}

#[contractimpl]
impl LedgerLensScoreContract {
    // ═════════════════════════════════════════════════════════════════════
    // Issue: Tiered gate fee schedule
    // ═════════════════════════════════════════════════════════════════════
    //
    // See `docs/operations/runbook.md`. `query_risk_gate_metered` is a new,
    // additive entry point that debits a consumer's prepaid gate credits
    // (above) per a governed volume-tiered fee schedule, then delegates to
    // the existing, unmodified `query_risk_gate_with_confidence` for the
    // actual gate decision — so this never changes the behavior or ABI of
    // `query_risk_gate` itself.

    /// Governance: replaces the fee-tier schedule. `tiers` must be sorted
    /// strictly ascending by `min_volume` (the first tier's `min_volume`
    /// need not be `0` — see `compute_gate_fee`'s "below first tier" case,
    /// which prices at `0`). Takes effect immediately for calls made from
    /// this point forward; does not and cannot retroactively change fees
    /// already charged for prior calls, since each call's fee is computed
    /// and debited synchronously at call time — there is no stored
    /// per-call record this could rewrite.
    ///
    /// # Errors
    /// - [`Error::InvalidFeeTierSchedule`] if `tiers` is not strictly
    ///   ascending by `min_volume`, exceeds `MAX_FEE_TIERS`, or any tier's
    ///   fee is negative or exceeds `MAX_GATE_FEE_TIER`.
    pub fn set_fee_tier_schedule(
        env: Env,
        admin_signers: Vec<Address>,
        tiers: Vec<FeeTier>,
    ) -> Result<(), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        if tiers.len() > crate::constants::MAX_FEE_TIERS {
            return Err(Error::InvalidFeeTierSchedule);
        }
        let mut prev_min_volume: Option<u32> = None;
        for i in 0..tiers.len() {
            let tier = tiers.get(i).unwrap();
            if tier.fee < 0 || tier.fee > crate::constants::MAX_GATE_FEE_TIER {
                return Err(Error::InvalidFeeTierSchedule);
            }
            if let Some(prev) = prev_min_volume {
                if tier.min_volume <= prev {
                    return Err(Error::InvalidFeeTierSchedule);
                }
            }
            prev_min_volume = Some(tier.min_volume);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        storage::set_fee_tier_schedule(&env, &tiers);
        events::fee_tier_schedule_set(&env, tiers.len());
        Ok(())
    }

    pub fn get_fee_tier_schedule(env: Env) -> Vec<FeeTier> {
        storage::get_fee_tier_schedule(&env)
    }

    /// Governance: sets the rolling window length (ledgers) used for
    /// per-consumer call-volume accounting.
    pub fn set_fee_tier_window_ledgers(
        env: Env,
        admin_signers: Vec<Address>,
        window_ledgers: u32,
    ) -> Result<(), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        if window_ledgers == 0 {
            return Err(Error::InvalidFeeTierSchedule);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        storage::set_fee_tier_window_ledgers(&env, window_ledgers);
        Ok(())
    }

    /// Governance: grants `consumer` a fee exemption (computed fee is
    /// always `0`, regardless of the tier schedule) until `expires_at`
    /// (ledger timestamp), tagged with an operator-defined `reason_code`
    /// (e.g. "public-good protocol", "internal testing"). Changeable only
    /// by governance; every change is auditable via
    /// [`events::fee_exemption_set`].
    ///
    /// # Errors
    /// - [`Error::InvalidExemptionExpiry`] if `expires_at` is not in the
    ///   future.
    pub fn set_fee_exemption(
        env: Env,
        admin_signers: Vec<Address>,
        consumer: Address,
        expires_at: u64,
        reason_code: u32,
    ) -> Result<(), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        if expires_at <= env.ledger().timestamp() {
            return Err(Error::InvalidExemptionExpiry);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        storage::set_fee_exemption(&env, &consumer, expires_at, reason_code);
        events::fee_exemption_set(&env, &consumer, expires_at, reason_code);
        Ok(())
    }

    /// Governance: revokes `consumer`'s exemption immediately (rather than
    /// waiting for it to expire naturally).
    pub fn clear_fee_exemption(
        env: Env,
        admin_signers: Vec<Address>,
        consumer: Address,
    ) -> Result<(), Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        Self::require_admin_auth(&env, &admin_signers)?;
        storage::clear_fee_exemption(&env, &consumer);
        Ok(())
    }

    /// Returns `Some((expires_at, reason_code))` if `consumer` currently
    /// has an unexpired exemption, else `None` (including if the stored
    /// record has expired but not yet been cleared — `query_risk_gate_metered`
    /// checks the expiry itself rather than relying on eager cleanup).
    pub fn get_fee_exemption(env: Env, consumer: Address) -> Option<(u64, u32)> {
        match storage::get_fee_exemption(&env, &consumer) {
            Some((expires_at, reason_code)) if expires_at > env.ledger().timestamp() => {
                Some((expires_at, reason_code))
            }
            _ => None,
        }
    }

    /// Metered, credit-funded variant of `query_risk_gate`. Computes the
    /// fee for this call from `consumer`'s current volume tier (or `0` if
    /// `consumer` has an active exemption), debits it from `consumer`'s
    /// prepaid gate-credit balance, then delegates to the unmodified
    /// `query_risk_gate_with_confidence` for the actual gate decision.
    ///
    /// `consumer` is the entity being billed — not necessarily `wallet`
    /// (the wallet being scored). `consumer.require_auth()` is required so
    /// only the consumer itself (or a contract acting with the consumer's
    /// authorization) can spend that consumer's credit.
    ///
    /// # Errors
    /// - [`Error::NotInitialized`] if the contract has no admin yet.
    /// - [`Error::InsufficientGateCredits`] if the computed fee exceeds
    ///   `consumer`'s current credit balance. The gate is *not* evaluated
    ///   in this case — callers must have funded credit before querying,
    ///   there is no partial/free fallback.
    pub fn query_risk_gate_metered(
        env: Env,
        consumer: Address,
        wallet: Address,
        asset_pair: Symbol,
        gate_threshold: u32,
    ) -> Result<bool, Error> {
        if !storage::has_admin(&env) {
            return Err(Error::NotInitialized);
        }
        consumer.require_auth();

        let fee = match Self::get_fee_exemption(env.clone(), consumer.clone()) {
            Some(_) => 0,
            None => {
                let volume_before_call = storage::advance_consumer_volume_window(&env, &consumer);
                let tiers = storage::get_fee_tier_schedule(&env);
                compute_gate_fee(&tiers, volume_before_call)
            }
        };

        if fee > 0 {
            storage::debit_gate_balance(&env, &consumer, fee)?;
            storage::sub_gate_credit_liability(&env, fee);
            storage::add_gate_credit_revenue(&env, fee);
        }
        events::gate_query_metered(&env, &consumer, fee);

        Ok(Self::query_risk_gate_with_confidence(env, wallet, asset_pair, gate_threshold, 0))
    }
}
