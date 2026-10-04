# Score Math (`ledgerlens-math`)

This document describes the shared, `no_std` fixed-point and statistics crate
`ledgerlens-math` that backs all score arithmetic in LedgerLens.

## Motivation

Score arithmetic (weighted aggregation, decay, interpolation, variance,
percentiles, fee computation) previously lived inside the contract crate and
could not be reused by the aggregator, the replay tool, the simulator, or
formal verification without pulling in contract dependencies. The math is now
extracted into a single audited implementation used everywhere.

## Crate layout

```
crates/ledgerlens-math/
  Cargo.toml        # no_std, no Soroban SDK dependency
  CHANGELOG.md
  src/
    lib.rs          # public API + crate docs
    fixed.rs        # fixed-point primitives
    stats.rs        # aggregation, variance, percentiles
    fee.rs          # fee computation
```
float_value = fixed / SCALE

**Key property:** All intermediate computations use integers; the result is truncated (not rounded) to match on-chain behavior. A simulator using rounding will diverge from the contract.

---

## Weighted Average

### Formula

The aggregate risk score is a weighted average of per-pair component scores:

$$\text{aggregate\_score} = \frac{\sum_{i=1}^{n} w_i \cdot s_i}{\sum_{i=1}^{n} w_i}$$

Where:
- $s_i$ = component score for pair $i$ (0–100)
- $w_i$ = weight assigned to pair $i$ (configurable per pair, defaults to 1)
- $n$ = number of distinct asset pairs the wallet has a score for

### Integer Implementation

In the contract (`compute_aggregate_score`), the computation is:

```rust
let mut weighted_sum: u64 = 0;
let mut weight_sum: u64 = 0;

for each (pair, score) in wallet.scores {
    let weight = get_pair_weight(pair);
    let decayed_weight = weight * decay_factor / SCALE;  // If decay is enabled
    
    let product = decayed_weight * score;
    weighted_sum += product;
    weight_sum += decayed_weight;
}

let aggregate_score = (weighted_sum / weight_sum) as u32;
```yaml

**Integer arithmetic notes:**
- `weighted_sum` and `weight_sum` are `u64` to prevent overflow when accumulating across up to 20 pairs.
- Multiplication is checked (`checked_mul`) — overflow returns an error.
- Division truncates toward zero (integer division in Rust).
- The final result is cast to `u32` and is guaranteed to be in 0–100 (since component scores are 0–100 and this is a weighted average).

### Off-Chain Simulation

To match on-chain results exactly:

```python
# Python reference implementation
SCALE = 1_000_000

def aggregate(pairs_and_scores, pair_weights, decay_factors=None):
    """
    pairs_and_scores: list of (pair_symbol, score) tuples
    pair_weights: dict[pair_symbol -> weight]
    decay_factors: dict[pair_symbol -> decay_factor] (each scaled by SCALE)
    """
    if not decay_factors:
        decay_factors = {}
    
    weighted_sum = 0
    weight_sum = 0
    
    for pair, score in pairs_and_scores:
        weight = pair_weights.get(pair, 1)
        decay = decay_factors.get(pair, SCALE)
        
        # Decayed weight: (weight * decay) / SCALE
        decayed_weight = (weight * decay) // SCALE
        
        # Accumulate
        product = decayed_weight * score
        weighted_sum += product
        weight_sum += decayed_weight
    
    if weight_sum == 0:
        # On-chain: get_aggregate_score returns Err(ScoreNotFound)
        raise ValueError("All weights are zero")
    
    # Truncate division
    aggregate_score = weighted_sum // weight_sum
    return aggregate_score

---

## Exponential Decay

### Formula

When a decay rate is configured (`set_decay_rate` with a non-zero numerator), every score is decayed by its age since submission, using exponential decay. The staleness window does **not** gate decay: a one-hour-old score is already decayed slightly. (Corrected in #1240 after differential testing; see `tools/reference-model/DISAGREEMENTS.md` D2/D4.)

$$\text{decay\_factor}(t) = e^{-\lambda \cdot t}$$

Where:
- $t$ = age in seconds since the score was submitted
- $\lambda$ = decay rate (numerator and denominator are configurable separately)
- Result is scaled by `SCALE` for fixed-point arithmetic

### Half-Life Interpretation

If you want scores to decay to half their impact after $T$ seconds:

$$\lambda = \frac{\ln(2)}{T} \approx \frac{0.693}{T}$$

For example, if $T = 30$ days = 2,592,000 seconds:
$$\lambda \approx 0.000000267$$

In fixed-point (scaled by $10^6$): $\lambda_{\text{scaled}} \approx 0.267$

### Integer Approximation (Taylor Series)

Soroban has no `exp()` function. The decay is approximated using a 4-term Taylor series:

$$e^{-x} \approx 1 - x + \frac{x^2}{2} - \frac{x^3}{6} + \frac{x^4}{24}$$

Where $x = \lambda \cdot t$ (scaled by `SCALE`).

**Accuracy:** For $x < 5$, this approximation achieves ~6 decimal places of precision (error < 0.01%).

### Integer Implementation

From `lib.rs` (`decay_fixed` function):

```rust
const SCALE: u64 = 1_000_000;

fn decay_fixed(age_secs: u64, lambda_num: u32, lambda_den: u32) -> u64 {
    if lambda_num == 0 {
        return SCALE;  // No decay
    }
    
    // Compute x_scaled = (num * age_secs * SCALE) / den
    let x_scaled = (lambda_num as u64)
        .checked_mul(age_secs)
        .and_then(|v| v.checked_mul(SCALE))
        .and_then(|v| v.checked_div(lambda_den as u64))
        .unwrap_or(0);
    
    // For large x, decay → 0
    if x_scaled >= 5 * SCALE {
        return 0;
    }
    
    // Taylor series: 1 - x + x²/2 - x³/6 + x⁴/24
    let x = x_scaled as i128;
    let s = SCALE as i128;
    
    let mut result = s;                    // Term 0: 1
    result -= x;                           // Term 1: -x
    result += (x * x) / (2 * s);           // Term 2: +x²/2
    result -= (x * x * x) / (6 * s * s);   // Term 3: -x³/6
    result += (x * x * x * x) / (24 * s * s * s);  // Term 4: +x⁴/24
    
    // Clamp to [0, SCALE]
    if result < 0 {
        0
    } else if result > s {
        SCALE
    } else {
        result as u64
    }
}
```yaml

### Off-Chain Reference

```python
import math

SCALE = 1_000_000

def decay_factor(age_secs, lambda_num, lambda_den):
    """
    Compute the decay factor e^(-lambda * age).
    lambda = lambda_num / lambda_den
    Returns the result scaled by SCALE.
    """
    if lambda_num == 0:
        return SCALE
    
    # Compute x_scaled = (lambda_num * age_secs * SCALE) / lambda_den
    x_scaled = (lambda_num * age_secs * SCALE) // lambda_den
    
    if x_scaled >= 5 * SCALE:
        return 0
    
    # Taylor series approximation
    x = x_scaled
    s = SCALE
    
    result = s                                      # 1
    result -= x                                     # -x
    result += (x * x) // (2 * s)                    # +x²/2
    result -= (x * x * x) // (6 * s * s)            # -x³/6
    result += (x * x * x * x) // (24 * s * s * s)   # +x⁴/24
    
    # Clamp
    result = max(0, min(result, s))
    return result

---

## Linear Interpolation

### Formula

When querying a score at a timestamp between two historical entries, linear interpolation is used:

$$\text{score}(t) = s_a + (t - t_a) \cdot \frac{s_b - s_a}{t_b - t_a}$$

Where:
- $(t_a, s_a)$ = earlier history entry (timestamp and score)
- $(t_b, s_b)$ = later history entry
- $t$ = query timestamp

### Integer Implementation

From `lib.rs` (`get_interpolated_score`):

```rust
pub fn get_interpolated_score(
    env: Env,
    wallet: Address,
    asset_pair: Symbol,
    timestamp: u64,
) -> u32 {
    let history = storage::get_score_history(&env, &wallet, &asset_pair);
    
    if history.is_empty() {
        return 0;
    }
    
    // Exact match: return stored value
    for entry in history {
        if entry.timestamp == timestamp {
            return entry.score;
        }
    }
    
    // Extrapolation: clamp to boundaries
    if timestamp <= history.first().timestamp {
        return history.first().score;
    }
    if timestamp >= history.last().timestamp {
        return history.last().score;
    }
    
    // Interpolation: find the bracketing pair
    for i in 0..(history.len() - 1) {
        let a = &history[i];
        let b = &history[i + 1];
        
        if a.timestamp <= timestamp && timestamp <= b.timestamp {
            let dt = (b.timestamp - a.timestamp) as i128;
            if dt == 0 {
                return a.score;
            }
            
            let num = (timestamp - a.timestamp) as i128 * (b.score as i128 - a.score as i128);
            return (a.score as i128 + num / dt) as u32;
        }
    }
    
    history.last().score
}
```yaml

**Integer arithmetic notes:**
- Numerator is `(timestamp - a.timestamp) * (b.score - a.score)` as `i128` to avoid overflow.
- Division truncates (integer division).
- Cast back to `u32` for the result.

---

## Overflow Handling

The contract uses checked arithmetic throughout the hot path to prevent silent overflows:

### Checked Operations

- **`get_aggregate_score`:**
  - `checked_mul(weight, decay_factor)` → error on overflow
  - `checked_div(SCALE)` → error on division by zero
  - `checked_mul(decayed_weight, score)` → error on overflow
  - `checked_add(weighted_sum, product)` → error on overflow
  
- **`decay_fixed`:**
  - `checked_mul` for `lambda_num * age_secs`
  - `checked_mul` for intermediate products
  - Saturating subtraction for negative results

### Error Propagation

When overflow is detected:
- Functions that compose checked operations return `Err(Error::ArithmeticOverflow)`.
- Callers must handle this error.
- On-chain, overflow is visible to integrators as an explicit error, preventing silent failures.

### Off-Chain Simulation

To avoid overflow in Python, use arbitrary-precision integers (Python 3 does this automatically for `int`). In other languages, use 128-bit or 256-bit integers for intermediate calculations.

---

## Staleness and Filtering

### Staleness Window

Scores older than the staleness window (default: `DEFAULT_STALENESS_WINDOW_SECS = 604,800` seconds = 7 days) are considered stale.

### Staleness Filtering in `get_effective_score`

1. Compute age: `age = current_timestamp - score_timestamp`
2. If `decay_rate != 0` (at any age; the staleness window is not consulted here):
   - Apply decay: `effective_score = raw_score * decay_factor(age) / SCALE` (truncated)
   - Set `decay_applied = true`
3. Otherwise:
   - `effective_score = raw_score`
   - Set `decay_applied = false`

### Embargo Filtering

Embargoed wallets are checked separately:
- `is_embargoed(wallet)` returns `true` if the wallet is on the embargo list.
- `get_effective_score` returns `Err(ScoreEmbargoed)` if the wallet is embargoed.
- `query_risk_gate` returns `false` if the wallet is embargoed.

---

## Asset-Class Policy Profiles

Different asset categories (stablecoins, volatile assets, thin markets, high-value pairs) warrant different risk-threshold policies. `get_effective_risk_threshold(asset_pair)` resolves the threshold to use for a pair:

1. If the pair has been assigned a class via `set_pair_asset_class(pair, class)` **and** that class has an override via `set_asset_class_policy(class, risk_threshold)`, the class override is returned.
2. Otherwise, the global `risk_threshold` (set via `set_risk_threshold`) is returned.

Lookup is a pure function of on-chain storage — same inputs always produce the same result — and pairs with no assigned class, or classes with no configured override, safely fall back to the global default rather than erroring. See `contracts/ledgerlens-score/src/test_asset_class_policy.rs` for fixtures covering the default-fallback and override-resolution paths.

---

## Precision Limits and Rounding

### Integer Truncation, Not Rounding

All divisions truncate toward zero. For example:
7 / 2 = 3  (not 3.5 rounded to 4)
```yaml

This behavior is deterministic and matches across platforms.

### Precision Loss

When computing:
aggregate = (weighted_sum / weight_sum)
```yaml

The result is truncated. For example, if the true average is 42.7, the contract returns 42. Off-chain simulators must use the same truncation to match.

### Decimal Precision

Fixed-point representation with `SCALE = 10^6` provides 6 decimal places. Values are stored as integers, so no floating-point rounding errors occur.

---

## Cross-Reference: Formula Documentation in Source Code

The following functions in `contracts/ledgerlens-score/src/lib.rs` reference this document:

- **`get_aggregate_score` (line ~1850):** See [§ Weighted Average](#weighted-average) for the formula and fixed-point implementation notes.
- **`get_effective_score` (line ~1601):** See [§ Staleness and Filtering](#staleness-and-filtering) and [§ Exponential Decay](#exponential-decay) for staleness filtering and decay logic.
- **`get_interpolated_score` (line ~1709):** See [§ Linear Interpolation](#linear-interpolation) for the formula and fixed-point implementation notes.
- **`decay_fixed` (line ~5340):** See [§ Exponential Decay](#exponential-decay) for the Taylor series approximation and fixed-point arithmetic.

---

## Off-Chain Simulation Checklist

When building an off-chain simulator (indexer, backend, analytics):

- [ ] Use integer arithmetic with the same `SCALE = 1,000,000` factor.
- [ ] Implement truncating division (not rounding).
- [ ] Use 64-bit or larger integers for intermediate calculations to prevent overflow.
- [ ] Implement the decay Taylor series with the same 4-term expansion.
- [ ] Handle edge cases: empty wallet lists, all-zero weights, invalid thresholds.
- [ ] Test against on-chain results with known inputs to verify precision.

---

---

## Monotonicity of Aggregate Score Under Pair Reweighting (#721)

When pair weights are changed while input scores remain fixed, the aggregate
score changes in predictable, monotone directions.

### Monotonicity Properties

| Property | Statement |
|---|---|
| M1 | Increasing the weight of a pair whose score is **above** the current aggregate **raises** (or preserves) the aggregate. |
| M2 | Increasing the weight of a pair whose score is **below** the current aggregate **lowers** (or preserves) the aggregate. |
| M3 | Setting all weights to zero is a degenerate case; the contract returns an error when `weight_sum = 0`. |
| M4 | A single pair with a very large weight dominates the aggregate: `agg → score_dominant` as `weight_dominant → ∞`. |
| M5 | If all pairs have the same score `S`, any positive reweighting leaves `agg = S`. |
| M6 | `max_pair_score` always equals the maximum individual score, independent of reweighting. |

### Worked Examples

**Example 1 — raising a high-score pair's weight:**
- Pairs: A (score=80, w=1), B (score=20, w=1) → `agg = floor((80+20)/2) = 50`
- Raise A's weight to 10: `agg = floor((10×80 + 1×20)/11) = floor(820/11) = 74` ✓ (increased)

**Example 2 — raising a low-score pair's weight:**
- Pairs: A (score=20, w=1), B (score=80, w=1) → `agg = 50`
- Raise A's weight to 10: `agg = floor((10×20 + 1×80)/11) = floor(280/11) = 25` ✓ (decreased)

### Test Coverage

Monotonicity properties are verified in
`contracts/ledgerlens-score/src/test_monotonicity_reweight.rs` using
deterministic unit tests with explicit expected values for each property.

---

## Confidence-Floor Semantics: Formal Truth Tables (#722)

The gate function `query_risk_gate_with_confidence` passes only when **all
three** of the following conditions hold simultaneously. The score check is
strict: a wallet passes only when its risk score is *below* the threshold,
since higher scores are more suspicious. (Corrected in #1240; earlier revisions
of these tables had the score comparison inverted. See
`tools/reference-model/DISAGREEMENTS.md` D1.)

PASS  iff  score       <  threshold
       AND confidence  >= query_conf
       AND confidence  >= global_min_confidence
```yaml

Where `global_min_confidence` is the admin-controlled floor set via
`set_global_min_confidence`.

### Table 1 — Score vs Threshold (confidence always passes)

| score | threshold | conf | query_conf | global_floor | result | reason |
|------:|----------:|-----:|-----------:|-------------:|:------:|--------|
|    60 |        70 |   90 |          0 |            0 | PASS   | score < threshold |
|    70 |        70 |   90 |          0 |            0 | FAIL   | score == threshold (strict boundary) |
|    69 |        70 |   90 |          0 |            0 | PASS   | score one below threshold |
|     0 |         0 |   90 |          0 |            0 | FAIL   | threshold 0 blocks every wallet |
|   100 |       100 |   90 |          0 |            0 | FAIL   | both max (score == threshold) |
|     0 |       100 |   90 |          0 |            0 | PASS   | score 0, threshold max |

### Table 2 — Confidence vs Per-Query Confidence Threshold

| score | threshold | conf | query_conf | global_floor | result | reason |
|------:|----------:|-----:|-----------:|-------------:|:------:|--------|
|    60 |        70 |   80 |         80 |            0 | PASS   | conf == query_conf (inclusive) |
|    60 |        70 |   79 |         80 |            0 | FAIL   | conf one below query_conf |
|    60 |        70 |   81 |         80 |            0 | PASS   | conf above query_conf |
|    60 |        70 |  100 |        100 |            0 | PASS   | conf == query_conf == max |
|    60 |        70 |   99 |        100 |            0 | FAIL   | conf one below max query_conf |
|    60 |        70 |    0 |          0 |            0 | PASS   | both zero |

### Table 3 — Confidence vs Global Minimum Confidence Floor

| score | threshold | conf | query_conf | global_floor | result | reason |
|------:|----------:|-----:|-----------:|-------------:|:------:|--------|
|    60 |        70 |   75 |          0 |           75 | PASS   | conf == global_floor (inclusive) |
|    60 |        70 |   74 |          0 |           75 | FAIL   | conf one below global_floor |
|    60 |        70 |   76 |          0 |           75 | PASS   | conf above global_floor |
|    60 |        70 |    0 |          0 |            0 | PASS   | floor is zero, never blocks |
|    60 |        70 |  100 |          0 |          100 | PASS   | conf == global_floor == max |

### Table 4 — Combined Constraints

| score | threshold | conf | query_conf | global_floor | result | reason |
|------:|----------:|-----:|-----------:|-------------:|:------:|--------|
|    60 |        70 |   85 |         80 |           75 | PASS   | all three conditions pass |
|    80 |        70 |   85 |         80 |           75 | FAIL   | score >= threshold |
|    60 |        70 |   79 |         80 |           75 | FAIL   | conf < query_conf |
|    60 |        70 |   74 |         70 |           75 | FAIL   | conf < global_floor |
|    60 |        70 |   74 |         80 |           75 | FAIL   | conf fails both conf checks |
|    99 |       100 |  100 |        100 |          100 | PASS   | score just below max threshold, confidences at max |

### Configuration Notes

- `global_min_confidence` is set admin-only via `set_global_min_confidence(floor: u32)`.
- Valid range: `[0, 100]`. Setting to 0 disables the floor (never blocks on confidence alone).
- The floor applies retroactively: raising it causes already-submitted scores with lower
  confidence to fail future gate queries without resubmission.

### Test Coverage

Truth tables are verified row-by-row in
`contracts/ledgerlens-score/src/test_confidence_floor_truth_tables.rs`.

---

## Model-Version Risk-Policy Compatibility (#723)

The active risk policy defines an allowlist of approved model versions.
Score submissions that carry an unapproved or retired version are rejected
deterministically at submission time.

### Version Lifecycle

                  register_model_version(v, delay)
                           │
                    delay elapsed?
                    ┌─── No ───→  Proposed  (not yet accepted)
                    │
                    └─── Yes ──→  Active    (accepted by risk policy)
                                      │
                              deprecate_model_version(v)
                                      │
                                  Deprecated  (permanently retired)
```yaml

### Compatibility Rules

| Registry state | Submitted version | Outcome |
|---|---|---|
| Empty (no versions registered) | any | ACCEPTED (fallback: no restriction) |
| Non-empty | Active version | ACCEPTED |
| Non-empty | Proposed version (delay not elapsed) | REJECTED |
| Non-empty | Deprecated version | REJECTED |
| Non-empty | Unknown version (never registered) | REJECTED |

### Read API

- `is_model_version_active(version: u32) -> bool` — returns `true` if and only if the version
  is in the Active state. Off-chain tooling should call this before submitting to avoid a
  wasted transaction.
- `get_model_versions() -> Vec<ModelVersionEntry>` — returns the full registry with each
  entry's `version`, `status`, and `metadata` bytes.

### ABI / Storage Notes

- The registry is stored under a persistent storage key (`MODEL_VERSIONS`).
- Deprecation is irreversible: a deprecated version cannot be re-activated.
- The maximum registry size is bounded by `MAX_MODEL_VERSIONS` (defined in `constants.rs`)
  to prevent unbounded storage growth.

### Test Coverage

Model-version policy compatibility is verified in
`contracts/ledgerlens-score/src/test_model_version_policy_compat.rs`.
Existing lifecycle tests live in
`contracts/ledgerlens-score/src/test_model_version.rs`.

---

## Bounded Drift Checks for Consecutive Score Updates (#724)

Consecutive score updates for the same `(wallet, asset_pair)` are checked
against a configurable drift threshold (the "jump threshold").  A score change
whose absolute delta exceeds the threshold is classified as a suspicious jump
and triggers an on-chain event.

The crate is `#![no_std]` and depends only on `core`. It must not depend on
`soroban-sdk` or any contract crate.

## Public API

All functions are pure: they take plain integers and return plain integers.
They never touch `Env`, storage, or events.

### Fixed-point

| Function | Contract |
| --- | --- |
| `mul_div(a, b, denom)` | Computes `a * b / denom` with 128-bit intermediate. Returns `None` on `denom == 0` or overflow. Rounds toward zero. |
| `mul_div_round(a, b, denom)` | As `mul_div`, but rounds half away from zero. |
| `clamp(x, lo, hi)` | Returns `x` clamped to `[lo, hi]`. Panics only if `lo > hi`. |

### Statistics

| Function | Contract |
| --- | --- |
| `weighted_mean(values, weights)` | Weighted mean with 128-bit accumulation. Returns `None` if lengths differ, are empty, or the total weight is zero. Rounds toward zero. |
| `decay(value, factor_bps, periods)` | Applies `factor_bps` decay per period. Returns `None` on overflow. |
| `interpolate(x, x0, x1, y0, y1)` | Linear interpolation. Returns `None` if `x1 == x0`. Rounds toward zero. |
| `variance(values)` | Population variance. Returns `None` if empty. Rounds toward zero. |
| `percentile(values, p_bps)` | Nearest-rank percentile, `p_bps` in `[0, 10_000]`. Returns `None` if empty or `p_bps > 10_000`. |

### Fees

| Function | Contract |
| --- | --- |
| `fee(amount, rate_bps)` | Computes `amount * rate_bps / 10_000`. Returns `None` on overflow. Rounds toward zero. |

## Overflow and rounding contracts

- Every fallible function returns `Option`; callers must handle `None`.
- Intermediate products use `i128`/`u128` to avoid silent wraparound.
- Rounding is documented per function and is always deterministic.
- No function panics on valid inputs; panics are reserved for programmer
errors (e.g. `clamp` with `lo > hi`).

## Differential testing

Before the old in-contract implementations were deleted, differential tests
ran the old and new implementations side by side on generated inputs and
asserted byte-identical results. These tests live in
`crates/ledgerlens-math/tests/differential.rs` and are exercised in a nightly
job over at least one million generated inputs.

## WASM size and CPU budget

Extraction is size-neutral or better: the contract crate now depends on
`ledgerlens-math` instead of carrying the math inline, and the crate is
`no_std` with no SDK dependency. Size and CPU budget reports are recorded in
the PR and show no regression.

## Consumers

`ledgerlens-math` is the single dependency for:

- the score contract,
- Kani harnesses,
- benchmarks,
- off-chain tools (aggregator, replay, simulator).

## Mutation testing

Mutation coverage is preserved on the moved code by pointing the mutation
testing configuration at `crates/ledgerlens-math/src/`.
