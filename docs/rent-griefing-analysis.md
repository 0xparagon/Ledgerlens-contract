# Rent Griefing via High-Cardinality Monitored Wallets

**Date:** 2026-07-26 · **Status:** Analysis complete — existing bound sufficient, no code change required beyond regression coverage

## Threat model

`submit_score` (`contracts/ledgerlens-score/src/lib.rs`) is gated by
authorized signers/service — it is not open to arbitrary public callers.
The realistic risk is therefore not an external attacker, but the
**off-chain scoring service (or a compromised/careless authorized signer)
submitting scores for a very large number of low-value wallet/asset-pair
combinations**: wallets with negligible balances or one-off pairs that
provide little risk-monitoring value but still occupy persistent storage
and consume rent indefinitely once written.

Each `(wallet, asset_pair)` combination creates:
- One `DataKey::Score` persistent entry (the score itself).
- One `DataKeyB::ScoreEntryLastTouchedLedger` persistent entry (rent-tracking
  touch marker, `storage.rs:174-182`).
- Optionally, a slot in the shared `ScoreEntryIndex` rent-management queue.

Persistent storage rent is proportional to entry count and TTL window, so
unbounded growth in distinct `(wallet, asset_pair)` combinations directly
increases the contract's ongoing rent burden and the cost of any full-index
sweep operation.

## Existing bound

`storage::reindex_entry_to_back` (`storage.rs:141-166`) already caps the
proactive rent-management index at `MAX_TRACKED_SCORE_ENTRIES` = 500
(`constants.rs:158`): once the index holds 500 distinct entries, new
distinct `(wallet, asset_pair)` combos are silently **not** added to the
index. This bounds `get_expiring_entries`'s sweep cost regardless of how
many distinct combinations have ever been submitted — see
`test_get_expiring_entries_short_circuits_full_index_scan` in
`test_ttl_rent_manager.rs`.

This is a resource-usage bound, not a submission bound: `set_score` still
accepts and persists writes for combinations beyond the 500-entry index.
Those entries still get their own TTL extended on write (self-renewing),
they just aren't visible to the admin's proactive-renewal sweep. In
practice this means low-value combinations beyond the cap are **not**
actively kept alive — if never resubmitted, they simply expire and archive
at their natural TTL, which already functions as passive cleanup.

## Proposed mitigation

No new enforcement code is required: the 500-entry index cap already
bounds the worst-case sweep cost, and TTL expiry already reclaims storage
for combinations that stop being resubmitted, at no extra rent cost to the
admin (Soroban does not charge for expired/archived entries). The
remaining risk is bounded persistent-storage growth from ever-increasing
distinct combinations that *are* still actively resubmitted (so they never
expire) — a genuine but low-value monitoring habit rather than an attack
enabled by a missing guard.

Two complementary strategies, ranked by cost/benefit, for a future issue if
the off-chain service's cardinality grows further:

1. **Prioritization (recommended, no ABI change):** have the off-chain
   service rank wallets by monitored value (balance, transaction volume)
   before submission and only resubmit the top `MAX_TRACKED_SCORE_ENTRIES`
   to keep the proactive index fully representative of what's actively
   monitored. Zero on-chain cost; purely an off-chain scheduling policy.
2. **Per-signer submission quota (higher cost, requires governance):** cap
   distinct new `(wallet, asset_pair)` combinations one signer can submit
   per epoch. Adds a new counter write per submission (extra rent per call)
   and a new timelocked governance parameter — justified only if a single
   compromised signer submitting thousands of low-value entries per epoch
   is judged a credible threat, which the signer-gating on `submit_score`
   currently makes unlikely.

## Compatibility impact

Documentation only for this issue. The regression test added alongside this
ADR (`test_high_cardinality_rent.rs`) exercises existing behavior; no
public ABI, event, error, or storage changes.

## Update: permissionless keeper reward for TTL extension

**Date:** 2026-09-29 · **Status:** Implemented — see `rent_relay_credits.rs`,
`test_keeper_rewards.rs`.

Liveness of score-entry TTLs previously depended entirely on the admin
periodically calling `extend_entry_ttls`. A new permissionless
`keeper_extend_entry_ttls(keeper, entries)` entry point lets *anyone* renew
dormant entries and be paid a small, governance-configured reward per entry
actually renewed — closing the "who runs the renewal cron job" single point
of failure without introducing a new griefing surface.

### Eligibility (bounds gaming)

An entry is reward-eligible only when its estimated remaining TTL (the same
conservative estimate `get_expiring_entries` already uses) is at or below a
governance-configured window (`KeeperRewardWindow`, default `0` — i.e.
already due, matching `get_expiring_entries`'s existing definition). This
directly prevents "extending entries that don't need it": an entry nowhere
near expiry is simply not eligible, full stop — `keeper_extend_entry_ttls`
skips it rather than renewing or paying for it.

"Splitting work to farm rewards" is closed by a property of the *existing*
TTL-tracking mechanism rather than new bookkeeping: renewing an entry resets
its last-touched ledger, so its estimated remaining TTL jumps straight back
to `SCORE_TTL_THRESHOLD` — far outside any sane reward window. The same
entry therefore cannot be renewed-and-paid again until it has genuinely
decayed back down near expiry, which is a full TTL cycle away. There is
deliberately no separate "last rewarded" record: reusing the touch marker
both is the renewal and is the anti-farming control, at zero extra storage
cost.

### Bounded payout

- Per-call batch size is capped at `KEEPER_BATCH_MAX` (reuses the existing
  `MAX_EXPIRING_ENTRIES_PER_CALL`), so the CLI can feed `get_expiring_entries`'s
  output straight in.
- Per-entry reward is capped at `MAX_KEEPER_REWARD_PER_ENTRY` regardless of
  what governance configures.
- An optional per-ledger cap (`KeeperRewardPerLedgerCap`) bounds total payout
  across *all* keeper calls in a single ledger, closing the "many small
  calls in one ledger" variant of batch-splitting.
- Funds come from a dedicated `KeeperRewardPool`, denominated in a
  governance-chosen token and topped up only by admin-authorized
  `fund_keeper_reward_pool` calls — structurally separate from gate-query
  credits (a different token/storage namespace entirely; see
  `docs/security/prepaid-gate-credits.md`), so user funds can never leak
  into keeper payouts.
- If the pool or per-ledger cap runs out mid-batch, still-eligible entries
  keep getting renewed (liveness is preserved) — they simply stop earning a
  reward, which costs the keeper nothing beyond the now-unrewarded
  transaction fee, so there's no incentive to keep spamming an empty pool.

### Keeper tooling

`tools/recovery`'s `keeper` subcommand takes a JSON array of candidate
`(wallet, asset_pair, estimated_ttl_remaining)` entries (the shape
`get_expiring_entries` naturally maps to off-chain) and emits the
most-urgent-first batch, capped at `KEEPER_BATCH_MAX`, ready to pass to
`keeper_extend_entry_ttls`.
