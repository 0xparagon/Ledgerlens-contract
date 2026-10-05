# Prepaid Gate-Query Credits — Design & Threat Model Addendum

**Date:** 2026-09-29 · **Status:** Implemented — see `rent_relay_credits.rs`
(the "Prepaid gate-query credits" and "Tiered gate fee schedule" sections),
`storage.rs`, `test_gate_credits.rs`, `test_gate_fee_tiers.rs`.

## Motivation

Paying a SEP-41 token transfer on every gate call is expensive and awkward
for a consumer that is itself a contract mid-transaction. This lets a
consumer deposit once and have metered queries (`query_risk_gate_metered`)
debit a running balance instead, with a token transfer only at deposit and
withdrawal time.

## Design

Three operations, each with a narrow, auditable purpose:

- **Deposit** (`deposit_gate_credits`): SEP-41 `transfer` from depositor to
  contract; credits `GateCreditBalance(depositor)` and
  `GateCreditLiabilityTotal` by the same amount.
- **Debit** (internal, called from `query_risk_gate_metered`): pure
  accounting — decrements `GateCreditBalance(consumer)` and
  `GateCreditLiabilityTotal`, increments `GateCreditRevenue`, by the fee
  amount. **No token transfer** — the tokens already sit in the contract
  from the original deposit; a debit just reclassifies them from "owed to
  depositor" to "governance-withdrawable revenue".
- **Withdraw** (`withdraw_gate_credits_request` + `withdraw_gate_credits`):
  two-step with an optional governance-configured delay
  (`GateCreditWithdrawalDelay`). SEP-41 `transfer` from contract back to
  depositor; decrements balance and liability by the same amount.

Governance separately withdraws accumulated revenue via
`withdraw_gate_credit_revenue`, scoped to `GateCreditRevenue` only.

## Why governance can never seize deposited credit

`GateCreditLiabilityTotal` (what's owed back to depositors) and
`GateCreditRevenue` (already-earned, governance-withdrawable) are two
disjoint O(1) counters. Every mutating operation touches exactly one of
them:

| Operation | Liability | Revenue | Token movement |
|---|---|---|---|
| `deposit_gate_credits` | `+= amount` | — | in |
| debit (metered query) | `-= fee` | `+= fee` | none |
| `withdraw_gate_credits` | `-= amount` | — | out |
| `withdraw_gate_credit_revenue` | — | `-= amount` | out |

There is no code path anywhere in the contract that decrements
`GateCreditLiabilityTotal` other than a depositor's own authorized
withdrawal or their own debited usage. `withdraw_gate_credit_revenue` is
hard-scoped to the `GateCreditRevenue` counter — it cannot be called with
an amount that draws down liability, because it never reads or writes that
counter at all. Withdrawals are also never gated by `is_paused` (the
contract-wide circuit breaker still blocks *new* deposits/queries via other
guards, but never blocks a depositor from getting their own unspent credit
back).

**Solvency invariant:** at every point, the contract's actual
`GateCreditToken` balance (`token_client.balance(contract_address)`) must
be `>= GateCreditLiabilityTotal + GateCreditRevenue`. Since deposits and
debits/revenue-withdrawals never move funds between namespaces (deposits
add to both actual balance and liability together; debits move funds
between liability and revenue with zero net effect on actual balance;
withdrawals of either kind remove from both actual balance and their own
counter together), the invariant is maintained by construction and is
covered by a randomized-operation-sequence test
(`test_gate_credits.rs::solvency_holds_across_random_operation_sequences`).

## Interaction with fee-token changes (governance)

`set_gate_credit_token` lets governance change which SEP-41 token is
accepted for *new* deposits. This does **not** retroactively reinterpret
existing balances — `GateCreditBalance`/`GateCreditLiabilityTotal`/
`GateCreditRevenue` are simple integers with no token identity attached to
each individual balance record; they are only meaningful relative to
*whichever* token is currently configured. Practically, this means:

- Changing the token while depositors hold balances in the old token
  creates a real operational hazard: those balances become effectively
  frozen from a token-transfer perspective (a withdrawal would attempt to
  transfer from the *new* token's contract, which never received those
  funds) unless the outgoing token is the same underlying asset.
- **Operator guidance:** before calling `set_gate_credit_token`, drain the
  system to zero total liability (every depositor has withdrawn, or the
  amounts are negligible and accepted as governance-forgiven — never
  governance-*seized*, since there is no code path to do that; the funds
  simply remain in the contract, still technically owed, until the token is
  changed back or a migration is performed off this base contract).
- This is documented as an operational runbook step rather than solved
  on-chain because solving it on-chain (e.g. an automatic migration sweep)
  would need to move funds between two different token contracts, which
  itself introduces a new class of trust and slippage assumptions out of
  scope for this issue.

## Interaction with fee tiers and quotas

The tiered fee schedule (`docs/operations/runbook.md`) determines *how
much* a debit charges per call; it never changes *how* a debit charges
(still pure accounting, still bounded by the depositor's balance). A
consumer with an empty balance is rejected with
`Error::InsufficientGateCredits` regardless of which fee tier they're in —
including the zero-fee tier or an active governance exemption, where the
computed fee is `0` and no debit (and no balance check) happens at all.

## Threat model addendum: reentrancy through token callbacks

SEP-41 tokens are Soroban contracts, and a malicious or non-standard token
configured via `set_gate_credit_token`/`set_keeper_reward_token`/
`set_relay_tip_token` could in principle attempt to call back into this
contract during its `transfer` implementation.

- **Deposit path:** the balance/liability counters are updated *after* the
  `transfer` call returns in `deposit_gate_credits`. A reentrant call during
  `transfer` would see the pre-deposit balance — it cannot observe or spend
  credit that hasn't been recorded yet. Soroban's storage model additionally
  means each contract invocation operates on its own instance of loaded
  storage within the host's transaction; there is no shared mutable global
  that a reentrant call could exploit to double-spend the *not-yet-written*
  credit.
- **Withdrawal path:** `withdraw_gate_credits` clears the withdrawal request
  and debits the balance/liability counters *before* calling `transfer`
  to send funds out. A reentrant call from a malicious token's `transfer`
  hook back into `withdraw_gate_credits` would find no pending request
  (already cleared) and fail with `Error::NoWithdrawalRequest` — the
  classic checks-effects-interactions ordering, applied specifically to
  guard against this.
- **Governance is the trust boundary:** ultimately, only a governance-approved
  token can be configured via `set_gate_credit_token` in the first place
  (admin M-of-N auth required). This is documented as defense-in-depth on
  top of that trust boundary, not a substitute for it — operators should
  only configure well-audited SEP-41 tokens.
