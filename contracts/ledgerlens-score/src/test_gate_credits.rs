#![cfg(test)]
//! Unit + property-style tests for prepaid gate-query credits
//! (see `docs/security/prepaid-gate-credits.md`).

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token::{StellarAssetClient, TokenClient},
    Address, Env, Vec,
};

use crate::{storage, Error, LedgerLensScoreContract, LedgerLensScoreContractClient};

struct Setup<'a> {
    env: Env,
    client: LedgerLensScoreContractClient<'a>,
    contract: Address,
    token: Address,
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
    Setup { env, client, contract, token }
}

/// Simulates what `query_risk_gate_metered` (added on top of this in the
/// next commit) does on the debit side: pure accounting, no token
/// transfer. Exercised directly here via `env.as_contract` since the public
/// metered-query entry point is scoped to a later commit on this branch.
fn debit(env: &Env, contract: &Address, consumer: &Address, fee: i128) -> Result<(), Error> {
    env.as_contract(contract, || {
        storage::debit_gate_balance(env, consumer, fee)?;
        storage::sub_gate_credit_liability(env, fee);
        storage::add_gate_credit_revenue(env, fee);
        Ok(())
    })
}

fn token_balance(env: &Env, token: &Address, holder: &Address) -> i128 {
    TokenClient::new(env, token).balance(holder)
}

// ── deposit ──────────────────────────────────────────────────────────────────

#[test]
fn deposit_requires_positive_amount() {
    let s = setup();
    let depositor = Address::generate(&s.env);
    assert_eq!(
        s.client.try_deposit_gate_credits(&depositor, &0),
        Err(Ok(Error::InvalidCreditAmount))
    );
    assert_eq!(
        s.client.try_deposit_gate_credits(&depositor, &-5),
        Err(Ok(Error::InvalidCreditAmount))
    );
}

#[test]
fn deposit_transfers_token_and_credits_balance() {
    let s = setup();
    let depositor = Address::generate(&s.env);
    StellarAssetClient::new(&s.env, &s.token).mint(&depositor, &1_000);

    let new_balance = s.client.deposit_gate_credits(&depositor, &400);
    assert_eq!(new_balance, 400);
    assert_eq!(s.client.get_gate_credit_balance(&depositor), 400);
    assert_eq!(s.client.get_gate_credit_liability_total(), 400);
    assert_eq!(token_balance(&s.env, &s.token, &depositor), 600);
    assert_eq!(token_balance(&s.env, &s.token, &s.contract), 400);
}

// ── debit (exact balance, zero balance) ───────────────────────────────────────

#[test]
fn exact_balance_debit_leaves_zero_and_moves_to_revenue() {
    let s = setup();
    let depositor = Address::generate(&s.env);
    StellarAssetClient::new(&s.env, &s.token).mint(&depositor, &1_000);
    s.client.deposit_gate_credits(&depositor, &250);

    debit(&s.env, &s.contract, &depositor, 250).unwrap();

    assert_eq!(s.client.get_gate_credit_balance(&depositor), 0);
    assert_eq!(s.client.get_gate_credit_liability_total(), 0);
    assert_eq!(s.client.get_gate_credit_revenue(), 250);
    // No token movement on debit — contract's actual holdings are unchanged.
    assert_eq!(token_balance(&s.env, &s.token, &s.contract), 250);
}

#[test]
fn zero_balance_debit_rejected() {
    let s = setup();
    let consumer = Address::generate(&s.env);
    assert_eq!(debit(&s.env, &s.contract, &consumer, 1), Err(Error::InsufficientGateCredits));
}

#[test]
fn debit_exceeding_balance_rejected_leaves_balance_unchanged() {
    let s = setup();
    let depositor = Address::generate(&s.env);
    StellarAssetClient::new(&s.env, &s.token).mint(&depositor, &1_000);
    s.client.deposit_gate_credits(&depositor, &100);

    assert_eq!(
        debit(&s.env, &s.contract, &depositor, 101),
        Err(Error::InsufficientGateCredits)
    );
    assert_eq!(s.client.get_gate_credit_balance(&depositor), 100);
}

// ── withdrawal ───────────────────────────────────────────────────────────────

#[test]
fn withdraw_without_request_rejected() {
    let s = setup();
    let depositor = Address::generate(&s.env);
    assert_eq!(
        s.client.try_withdraw_gate_credits(&depositor),
        Err(Ok(Error::NoWithdrawalRequest))
    );
}

#[test]
fn withdraw_before_delay_elapsed_rejected() {
    let s = setup();
    s.client.set_gate_credit_withdrawal_delay(&Vec::new(&s.env), &3_600);
    let depositor = Address::generate(&s.env);
    StellarAssetClient::new(&s.env, &s.token).mint(&depositor, &1_000);
    s.client.deposit_gate_credits(&depositor, &500);
    s.client.withdraw_gate_credits_request(&depositor, &500);

    assert_eq!(
        s.client.try_withdraw_gate_credits(&depositor),
        Err(Ok(Error::WithdrawalNotYetUnlocked))
    );
}

#[test]
fn withdraw_after_delay_elapsed_succeeds() {
    let s = setup();
    s.client.set_gate_credit_withdrawal_delay(&Vec::new(&s.env), &3_600);
    let depositor = Address::generate(&s.env);
    StellarAssetClient::new(&s.env, &s.token).mint(&depositor, &1_000);
    s.client.deposit_gate_credits(&depositor, &500);
    s.client.withdraw_gate_credits_request(&depositor, &500);

    s.env.ledger().set_timestamp(s.env.ledger().timestamp() + 3_601);
    let withdrawn = s.client.withdraw_gate_credits(&depositor);
    assert_eq!(withdrawn, 500);
    assert_eq!(s.client.get_gate_credit_balance(&depositor), 0);
    assert_eq!(token_balance(&s.env, &s.token, &depositor), 1_000);
}

#[test]
fn zero_delay_allows_immediate_withdrawal() {
    let s = setup(); // default delay is 0
    let depositor = Address::generate(&s.env);
    StellarAssetClient::new(&s.env, &s.token).mint(&depositor, &1_000);
    s.client.deposit_gate_credits(&depositor, &500);
    s.client.withdraw_gate_credits_request(&depositor, &500);
    let withdrawn = s.client.withdraw_gate_credits(&depositor);
    assert_eq!(withdrawn, 500);
}

#[test]
fn withdrawal_succeeds_even_while_contract_is_paused() {
    let s = setup();
    let depositor = Address::generate(&s.env);
    StellarAssetClient::new(&s.env, &s.token).mint(&depositor, &1_000);
    s.client.deposit_gate_credits(&depositor, &500);
    s.client.withdraw_gate_credits_request(&depositor, &500);

    s.client.pause(&Vec::new(&s.env));

    // Unspent credit must always be withdrawable, regardless of the
    // contract-wide pause circuit breaker.
    let withdrawn = s.client.withdraw_gate_credits(&depositor);
    assert_eq!(withdrawn, 500);
}

// ── revenue withdrawal is scoped and cannot touch liability ──────────────────

#[test]
fn revenue_withdrawal_cannot_exceed_earned_revenue() {
    let s = setup();
    let depositor = Address::generate(&s.env);
    StellarAssetClient::new(&s.env, &s.token).mint(&depositor, &1_000);
    s.client.deposit_gate_credits(&depositor, &1_000);
    debit(&s.env, &s.contract, &depositor, 100).unwrap();

    let recipient = Address::generate(&s.env);
    assert_eq!(
        s.client.try_withdraw_gate_credit_revenue(&Vec::new(&s.env), &recipient, &101),
        Err(Ok(Error::ArithmeticOverflow))
    );
    // Exactly the earned revenue succeeds.
    s.client.withdraw_gate_credit_revenue(&Vec::new(&s.env), &recipient, &100);
    assert_eq!(token_balance(&s.env, &s.token, &recipient), 100);
    // The depositor's remaining 900 liability is completely untouched.
    assert_eq!(s.client.get_gate_credit_balance(&depositor), 900);
}

// ── solvency invariant across randomized operation sequences ─────────────────

/// Minimal deterministic xorshift PRNG — avoids adding a `proptest`
/// dev-dependency just for one test while still exercising many
/// pseudo-random operation orderings/amounts deterministically.
struct Xorshift(u64);
impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

#[test]
fn solvency_holds_across_random_operation_sequences() {
    let s = setup();
    let mut rng = Xorshift(0x1234_5678_9abc_def0);

    let depositors: Vec<Address> =
        Vec::from_array(&s.env, [Address::generate(&s.env), Address::generate(&s.env), Address::generate(&s.env)]);
    for d in depositors.iter() {
        StellarAssetClient::new(&s.env, &s.token).mint(&d, &1_000_000);
    }

    for _ in 0..300 {
        let who = depositors.get(rng.range(depositors.len() as u64) as u32).unwrap();
        match rng.range(3) {
            0 => {
                let amount = (rng.range(500) as i128) + 1;
                s.client.deposit_gate_credits(&who, &amount);
            }
            1 => {
                let balance = s.client.get_gate_credit_balance(&who);
                if balance > 0 {
                    let fee = (rng.range(balance as u64) as i128) + 1;
                    let fee = fee.min(balance);
                    debit(&s.env, &s.contract, &who, fee).unwrap();
                }
            }
            _ => {
                let balance = s.client.get_gate_credit_balance(&who);
                if balance > 0 {
                    let amount = (rng.range(balance as u64) as i128) + 1;
                    let amount = amount.min(balance);
                    s.client.withdraw_gate_credits_request(&who, &amount);
                    s.client.withdraw_gate_credits(&who);
                }
            }
        }

        let liability = s.client.get_gate_credit_liability_total();
        let revenue = s.client.get_gate_credit_revenue();
        let actual_holdings = token_balance(&s.env, &s.token, &s.contract);
        assert!(
            actual_holdings >= liability + revenue,
            "insolvency: holdings={actual_holdings} < liability({liability}) + revenue({revenue})"
        );
        // No fees ever leave the contract in this simulation (no
        // withdraw_gate_credit_revenue calls), so the two counters must
        // account for the contract's entire token balance exactly.
        assert_eq!(actual_holdings, liability + revenue);
    }
}
