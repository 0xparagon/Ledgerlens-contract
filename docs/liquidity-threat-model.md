# Threat model: score manipulation incentives in liquidity protocols

This document focuses on AMM and lending integrations that consume
`query_risk_gate` or `query_risk_gate_with_confidence`.

## Current concrete behavior

- Missing score: gate fails closed
- Embargoed wallet: gate fails closed
- Pending finality-buffer score: downstream gate still sees the prior live
  score, or no score at all if none was committed yet
- Low-confidence score: `query_risk_gate_with_confidence` fails closed when
  `score.confidence < max(call_min_confidence, global_min_confidence)`
- Raw `query_risk_gate` does not itself encode a confidence floor

## Attack scenarios

### 1. Timing attack against finality windows

Scenario:

1. Attacker obtains or submits a low-risk score.
2. Score is held behind a non-zero `finality_buffer`.
3. Attacker attempts to trade or borrow before the score is committed.

Impact:

- If the integrator incorrectly assumes pending scores are already live, it may
  admit activity based on nonexistent state.

Current mitigation:

- The contract fails closed for unknown wallets and does not expose pending
  scores through the normal risk-gate path.

Tests:

- `amm_swap_rejected_while_safe_score_is_still_pending_finality`
- `lending_borrow_rejected_while_safe_score_is_still_pending_finality`

### 2. Confidence-floor evasion

Scenario:

1. Attacker obtains a low raw score with weak model confidence.
2. Protocol uses `query_risk_gate` instead of the confidence-aware variant.

Impact:

- Risk gate may admit an economically important action on low-quality evidence.

Current mitigation:

- `query_risk_gate_with_confidence`
- `global_min_confidence`
- Existing composability tests for low-confidence rejection

Operator guidance:

- AMMs with compliance or sanctions concerns should still prefer the
  confidence-aware gate.
- Lending protocols should not use raw `query_risk_gate` for credit decisions.

### 3. Whitewashing via rapid re-score

Scenario:

1. High-risk wallet receives a historically high score.
2. Compromised signer attempts to overwrite it with an artificially safe score.

Impact:

- Borrow or LP admission can occur before manual review.

Current mitigation:

- `score_floor_policy`
- `cooldown`
- `adaptive_rate_limit`
- multisig / threshold attestation controls

Residual risk:

- If operators leave the score floor disabled and allow aggressive cooldown
  settings, the window for exploitation expands materially.

### 4. Stale-safe-score carry trade

Scenario:

1. Wallet had a previously safe score during benign behavior.
2. Behavior deteriorates off-chain, but no fresh score arrives.
3. Integrator continues trusting an old score without appropriate freshness
   expectations.

Impact:

- Capital access granted against outdated evidence.

Current mitigation:

- `staleness_window`
- `get_effective_score` for integrations that explicitly care about stale-score
  penalty semantics

Follow-up note:

- `query_risk_gate` itself is a threshold gate, not a freshness policy engine.
  Integrators with strict freshness requirements should document an additional
  freshness check in their own flow.

## Recommended integration posture

- AMM swaps:
  - acceptable to use raw gate only when the venue tolerates lower assurance
  - prefer non-zero `finality_buffer` plus clear operator review
- Liquidity provision:
  - use confidence-aware gate and non-zero global confidence floor
- Lending:
  - use confidence-aware gate
  - keep `score_floor_policy` enabled
  - keep cooldown and adaptive rate-limit settings conservative

## Agent-based adversarial simulation of gate-threshold gaming

Threat models argue about incentives; the simulator in `tools/` quantifies them.
It runs an agent-based simulation where wash traders adapt to the published gate
thresholds, staleness windows and cooldowns, and measures how much manipulation
the system tolerates before the score reacts. The simulation drives a simplified
detection pipeline whose output is fed into the real contract test environment
(the Soroban contract test harness), so results reflect the actual gate logic
rather than a re-implementation.

### Agents

- **Honest traders** — organic volume, no attempt to game the gate.
- **Wash cycles** — self-dealing round trips that inflate apparent activity.
- **Threshold-hugging** — agents that keep each wallet's score just under the
  published gate threshold, exploiting the gap between the threshold and the
  detection boundary.
- **Splitting** — the same economic actor spread across many wallets and pairs
  to stay below per-wallet and per-pair limits.

### Parameterisation by contract configuration

The simulation is parameterised directly by contract configuration — gate
thresholds, `staleness_window`, `cooldown`, `finality_buffer`,
`score_floor_policy` and `adaptive_rate_limit` — so governance can evaluate a
proposed change before adoption. A proposed parameter change is expressed as a
config diff and run against the same seeded workload as the baseline.

### Reproducible reports

Runs are seeded and deterministic. Each report records the seed and emits:

- **time-to-detection** — ticks until the pipeline reacts to an attack.
- **exposure window** — ticks during which the attacker can act before reaction.
- **false-block rate** — honest traders incorrectly blocked by the gate.
- **attacker gains** — value extracted before detection.

### Canned scenarios

At least three canned attack scenarios ship with the tooling, plus one defence
comparison:

1. **Wash-cycle inflation** — sustained self-dealing to lift apparent volume.
2. **Threshold-hugging** — agents pinned just under the gate threshold.
3. **Wallet/pair splitting** — one actor fragmented across wallets and pairs.

Defence comparison: the same seeded workload is run with the baseline config
and with a hardened config (tighter `staleness_window`, non-zero
`finality_buffer`, enabled `score_floor_policy`) to show the delta in
time-to-detection, exposure window, false-block rate and attacker gains.

### Acceptance criteria

- Deterministic runs with recorded seeds and a golden report test.
- A documented example evaluating a real parameter change.
- Findings that reveal weaknesses are filed as separate issues with references.

### Documented example: evaluating a real parameter change

To evaluate a proposed change, run the simulator twice with the same seed —
once with the current config and once with the proposed config — and diff the
reports. For example, tightening `staleness_window` and enabling a non-zero
`finality_buffer` should reduce the exposure window for the stale-safe-score
carry trade (scenario 4 above) at the cost of a higher false-block rate for
honest traders. The golden report test pins the baseline output so any drift in
the detection pipeline or gate logic is caught in CI.

## Compatibility impact

- No existing gate ABI changed
- The new tests only document and lock in fail-closed timing semantics
- The simulator is tooling only and does not alter the public ABI, storage
  layout, events or error enum
