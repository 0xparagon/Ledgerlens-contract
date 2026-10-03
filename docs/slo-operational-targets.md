# Operational SLOs — Score Freshness & Risk-Gate Availability

Concrete, measurable operational targets for the off-chain service and on-chain
`ledgerlens-score` contract. These SLOs turn the product expectation "scores
should be trustworthy and the gate should be up" into numbers an operator can
alert on.

> **Source of truth for defaults:** [`contracts/ledgerlens-score/src/constants.rs`](../contracts/ledgerlens-score/src/constants.rs)

## Out of scope (explicit)

- No on-chain SLO enforcement is added. The contract already fails closed via
  staleness/heartbeat checks (`is_service_alive`, `InvalidStalenessWindow`);
  this document defines the **off-chain monitoring targets** around that
  existing behavior. It does not change contract logic.
- Does not cover throughput/gas-cost targets — see
  [`docs/wasm-size-budget.md`](wasm-size-budget.md) for size budgets.
- Does not define GrantFox/campaign-specific dashboards.

## 1. Score freshness

| Indicator (SLI) | Definition | Objective (SLO) | Alert threshold | Source |
|---|---|---|---|---|
| Heartbeat age | `now - get_last_service_activity()` | ≤ `DEFAULT_HEARTBEAT_ALERT_THRESHOLD_SECS` (1h) for 99.5% of ledger closes/month | Page when `is_service_alive() == false` | `constants::DEFAULT_HEARTBEAT_ALERT_THRESHOLD_SECS` |
| Score staleness | `now - score.timestamp` per asset pair | ≤ `DEFAULT_STALENESS_WINDOW_SECS` (7d) for 99.9% of reads | Warn at 80% of window, page at 100% | `constants::DEFAULT_STALENESS_WINDOW_SECS` |
| Oracle staleness (failover) | `now - oracle.updated_at` | ≤ `DEFAULT_ORACLE_STALENESS_THRESHOLD_SECS` (1h) | Page on breach — failover path (`FAILOVER_STALENESS_WINDOW`) engages at 1h | `constants::DEFAULT_ORACLE_STALENESS_THRESHOLD_SECS`, `FAILOVER_STALENESS_WINDOW` |

## 2. Risk-gate availability

| Indicator (SLI) | Definition | Objective (SLO) | Alert threshold | Source |
|---|---|---|---|---|
| Gate uptime | % of ledger closes where `is_paused() == false` for the queried pair | ≥ 99.95%/month, excluding operator-initiated maintenance pauses | Page immediately on any `paused` event not preceded by a scheduled-maintenance annotation | `paused` / `unpaused` events (`events.rs`) |
| Gate enforcement mode | `gate_enf` (strict mode) consistency | 100% — strict mode must not silently toggle off | Page on any `gate_enf` event | `events.rs:561` |
| Pair-level pauses | Count of individually paused pairs (`is_pair_paused`) | < 5% of active pairs paused at any time outside incident response | Warn at 5%, page at 10% | `pr_pause` event |

## 3. Event indexer freshness (reference indexer, #1212)

The reference event indexer ingests Soroban RPC contract events into Postgres
with reorg-safe cursors and backfill. Its health is measured by the same
freshness discipline as scores: an indexer that silently falls behind is an
outage even when the gate is up.

| Indicator (SLI) | Definition | Objective (SLO) | Alert threshold | Source |
|---|---|---|---|---|
| Ledger lag | `latest_ledger - last_processed_ledger` | ≤ 5 ledgers for 99.5% of minutes/month | Warn at 5, page at 25 | `indexer_health.ledger_lag` |
| Last processed ledger | `indexer_health.last_processed_ledger` monotonicity | Must advance every retention window; never regress | Page on regression or no advance for 2× poll interval | `indexer_health.last_processed_ledger` |
| Gap count | Number of detected missed-ledger ranges not yet backfilled | 0 sustained; any open gap is an incident | Warn on first gap, page if unresolved past one retention window | `indexer_health.gap_count` |

Retention-window awareness: Soroban RPC keeps limited history, so a gap that
ages past the retention window can no longer be filled from RPC and must be
backfilled from the archive source. The gap-count SLO is therefore bounded by
the retention window — an unresolved gap approaching that boundary escalates
from warn to page.

## 4. What "fail closed" means for these SLOs

Breaching a freshness SLO **must not** be silently absorbed by the contract —
`is_service_alive() == false` and stale scores are expected to surface as
`ScoreNotFound`/read-side rejections rather than serving expired data. These
SLOs measure how quickly operators detect and respond to that fail-closed
state; they do not relax it. See [`docs/errors.md`](errors.md) for the
authoritative error semantics.

## 5. Alert routing

Alerts on the above thresholds feed the severity classification in
[`docs/incident-severity-classification.md`](incident-severity-classification.md)
(added under a companion change) and the on-call runbooks in `docs/runbooks/`.
