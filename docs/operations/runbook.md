# LedgerLens Operator Runbook & Recovery Guide

This runbook details standard operating procedures, diagnostic health signals, and emergency recovery steps for operators managing `LedgerLens` contract deployments on Soroban.

---

## Tiered gate query fees

**Date:** 2026-09-29 · **Status:** Implemented — see `rent_relay_credits.rs`
(`query_risk_gate_metered`), `fee_schedule.rs`, `test_gate_fee_tiers.rs`.

### Overview

`query_risk_gate` (the original entry point) remains a free, side-effect-free
read — unchanged. `query_risk_gate_metered` is a new, additive entry point
that debits a `consumer`'s prepaid gate credits (see
`docs/security/prepaid-gate-credits.md`) according to a governed,
volume-tiered fee schedule, then performs the same gate check.

### Tier model

Governance configures an ordered list of `FeeTier { min_volume, fee }`
entries via `set_fee_tier_schedule`, sorted ascending by `min_volume`. Each
consumer has a bounded, O(1) rolling-window call-volume record
(`ConsumerVolumeWindow`): `(window_start_ledger, calls_in_window)`, reset to
`(now, 0)` once `FeeTierWindowLedgers` ledgers have elapsed since the window
started (default ~1 day at Stellar's ~5s average ledger close time). The fee
for a call is the highest tier whose `min_volume` is `<=` the consumer's
call count *before* this call — see `fee_schedule::compute_gate_fee`, a pure
function with no storage/`Env` access.

### Worked example

Schedule (a typical "cheaper as you use more" shape — the tier boundaries
and direction are entirely a governance choice, not hard-coded):

| Tier | `min_volume` | `fee` (stroops) |
|---|---|---|
| 0 | 0 | 100 |
| 1 | 10 | 50 |
| 2 | 100 | 10 |

For a consumer starting a fresh window:

- Calls 1–10 (volume-before-call 0–9): tier 0, **100** stroops each = 1,000 total.
- Calls 11–100 (volume-before-call 10–99): tier 1, **50** stroops each = 4,500 total.
- Call 101 onward (volume-before-call >= 100): tier 2, **10** stroops each.

If the consumer's window rolls over (no calls for `FeeTierWindowLedgers`
ledgers, or the window naturally expires), the next call's volume-before-call
resets to `0` and pricing restarts at tier 0 — this is intentional: the
schedule prices *recent* usage, not lifetime usage.

### Exemptions

`set_fee_exemption(consumer, expires_at, reason_code)` grants `consumer` a
`0`-fee exemption (public-good protocols, internal testing, etc.) until
`expires_at`. Exempt consumers skip volume-window accounting entirely for
the exempt period — their tier position doesn't advance while exempt, so
they resume at whatever volume they'd reached once the exemption lapses (or
at `0` if their window has separately rolled over in the meantime).
`get_fee_exemption` returns `None` once `expires_at` has passed even if
`clear_fee_exemption` hasn't been called yet — expiry is self-enforcing, not
dependent on a cleanup transaction.

### Rounding & overflow

None — see `fee_schedule::compute_gate_fee`'s doc comment. Tiers are flat
per-call amounts, not a proportional rate, so there is no division and
nothing to round; the only arithmetic is `u32` volume comparisons, which
cannot overflow.

### No retroactive fee changes

Each call's fee is computed and debited synchronously, at call time, from
whatever schedule is configured *then*. There is no stored per-call fee
record for a later governance action to rewrite — changing
`set_fee_tier_schedule` only affects calls made after the change lands.

## 