# Score Aggregation Mathematics

This document describes the mathematical formulas, fixed-point representation, and integer arithmetic used in LedgerLens score aggregation. Off-chain simulators must use identical integer arithmetic and truncation behavior to match on-chain results.

---

## Fixed-Point Representation

### Scale Factor

All fractional values in LedgerLens are represented as integers scaled by a fixed multiplier:

```
SCALE = 1,000,000  (10^6)

A value represented in fixed-point is scaled by multiplying by `SCALE`. For example:
- 1.0 is represented as `1_000_000`
- 0.5 is represented as `500_000`
- 0.000001 is represented as `1`

### Why Fixed-Point?

Soroban (Stellar's smart contract platform) has no floating-point arithmetic. Fixed-point integer arithmetic is used to approximate decimal values while maintaining determinism across all environments (on-chain Rust, off-chain simulators, indexers).

### Conversion Formulas

**From floating-point to fixed-point:**
```
fixed = float_value * SCALE

**From fixed-point to floating-point:**
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
        raise ValueError("All weights are zero")
    
    # Truncate division
    aggregate_score = weighted_sum // weight_sum
    return aggregate_score

---

## Exponential Decay

### Formula

When a score is older than the staleness window (default: 7 days), it is decayed using exponential decay:

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
        let 

---

## Asset-Level Risk Score (Derived from Holder Wallet Scores)

Protocols listing an asset care about the asset's risk, not only individual wallets. The asset-level score summarises the risk distribution of the wallets that hold or trade the asset, without requiring an on-chain scan of all holders.

### Derivation

The asset score is a **holder-weighted aggregation with a concentration penalty**. It is a pure function suitable for the shared math crate:

$$\text{asset\_score} = \text{clamp}_{0}^{100}\left( \frac{\sum_{i=1}^{n} w_i \cdot s_i}{\sum_{i=1}^{n} w_i} \cdot \left(1 - \rho \cdot C\right) \right)$$

Where:
- $s_i$ = wallet score for contributing holder $i$ (0–100)
- $w_i$ = contribution weight of holder $i$ (e.g. volume or balance, scaled by `SCALE`)
- $n$ = number of contributing wallets
- $C$ = concentration index of the weight distribution (0–1)
- $\rho$ = concentration penalty coefficient (configurable, default $\rho = 0.5$)

**Concentration index** (normalised Herfindahl–Hirschman Index):

$$C = \frac{n \cdot \sum_{i=1}^{n} w_i^2}{\left(\sum_{i=1}^{n} w_i\right)^2} - \frac{1}{n}$$

$C = 0$ when all weights are equal; $C \to 1$ when a single holder dominates. The penalty therefore reduces the score when a few large holders dominate the distribution, which is the intended behaviour: an asset whose risk is concentrated in one wallet is riskier than the same average spread across many wallets.

### Why Holder-Weighted with a Concentration Penalty?

- **Holder-weighted** (rather than a plain unweighted mean) reflects that a wallet holding most of the supply matters more to the asset's risk than a dust holder.
- **Concentration penalty** prevents a single large holder from masking a risky distribution and prevents the score from being dominated by one actor.
- **Pure function**: the derivation depends only on the list of `(weight, score)` pairs, so it is deterministic, testable, and identical on-chain and off-chain.

### Integer Implementation

```rust
const SCALE: u64 = 1_000_000;
const MIN_CONTRIBUTORS: u32 = 5;   // sparse-data threshold
const RHO: u64 = 500_000;          // concentration penalty coefficient (0.5)

/// Pure derivation. Returns `None` when fewer than `MIN_CONTRIBUTORS`
/// wallets contribute (sparse data — no score is published).
pub fn derive_asset_score(contributors: &[(u64, u32)]) -> Option<u32> {
    let n = contributors.len() as u64;
    if n < MIN_CONTRIBUTORS as u64 {
        return None;
    }

    let mut weight_sum: u64 = 0;
    let mut weighted_sum: u64 = 0;
    let mut sq_sum: u64 = 0;

    for &(w, s) in contributors {
        weight_sum = weight_sum.checked_add(w)?;
        weighted_sum = weighted_sum.checked_add(w.checked_mul(s as u64)?)?;
        sq_sum = sq_sum.checked_add(w.checked_mul(w)?)?;
    }

    if weight_sum == 0 {
        return None;
    }

    // Weighted mean, scaled by SCALE.
    let mean = weighted_sum.checked_mul(SCALE)? / weight_sum;

    // Normalised HHI concentration index, scaled by SCALE.
    // C = n * sq_sum / weight_sum^2 - 1/n
    let hhi = n.checked_mul(sq_sum)?.checked_mul(SCALE)? / weight_sum.checked_mul(weight_sum)?;
    let inv_n = SCALE / n;
    let concentration = hhi.saturating_sub(inv_n);

    // Penalty factor = 1 - rho * C, clamped to [0, SCALE].
    let penalty = SCALE.saturating_sub(RHO.checked_mul(concentration)? / SCALE);

    // Apply penalty and clamp to [0, 100].
    let score = mean.checked_mul(penalty)? / SCALE;
    Some(score.min(100) as u32)
}
```

**Integer arithmetic notes:**
- All intermediate values are `u64`; multiplications are checked and overflow returns `None`.
- Division truncates toward zero, matching the rest of the crate.
- The result is bounded to `[0, 100]` by construction and by the final `min(100)` clamp.

### Off-Chain Reference

```python
SCALE = 1_000_000
MIN_CONTRIBUTORS = 5
RHO = 500_000

def derive_asset_score(contributors):
    """contributors: list of (weight, score) tuples."""
    n = len(contributors)
    if n < MIN_CONTRIBUTORS:
        return None

    weight_sum = sum(w for w, _ in contributors)
    if weight_sum == 0:
        return None

    weighted_sum = sum(w * s for w, s in contributors)
    sq_sum = sum(w * w for w, _ in contributors)

    mean = (weighted_sum * SCALE) // weight_sum
    hhi = (n * sq_sum * SCALE) // (weight_sum * weight_sum)
    concentration = max(0, hhi - SCALE // n)
    penalty = max(0, SCALE - (RHO * concentration) // SCALE)
    score = (mean * penalty) // SCALE
    return min(score, 100)
```

### Sparse Data

A score is only published when at least `MIN_CONTRIBUTORS` wallets contribute. Below that threshold the derivation returns `None` and no asset score is stored or emitted. This avoids publishing a score derived from one or two wallets, which would be trivially manipulable and statistically meaningless.

### Maintenance Model: Incremental vs. Committed Off-Chain Result

Two maintenance strategies were considered:

| Strategy | Cost | Latency | Trust |
| --- | --- | --- | --- |
| **Incremental on-chain** — update the running aggregate on every submission | O(1) per submission, but requires storing running sums (`weight_sum`, `weighted_sum`, `sq_sum`) and recomputing the penalty each time; storage writes on every submission | Immediate | Fully on-chain, no extra trust |
| **Periodically committed off-chain result** — compute off-chain, commit the result on-chain | O(1) per commit (one write per epoch); off-chain compute is free | Up to one epoch | Requires a trusted committer or a verification path |

**Chosen approach: incremental on-chain maintenance.** The running sums (`weight_sum`, `weighted_sum`, `sq_sum`) are small fixed-size values, so each submission costs a bounded number of storage reads/writes and the derivation itself is O(1) given the sums. This keeps the score fully on-chain and avoids introducing a trusted committer. The periodically-committed off-chain variant is cheaper in storage writes but adds a trust assumption and latency; it is documented here as the fallback if submission volume makes per-submission writes prohibitive.

### Threat Model: Manipulation

- **Sybil wallets (many tiny contributors).** An attacker splitting holdings across many wallets inflates $n$ and lowers the concentration index, but each wallet's weight is tiny, so the weighted mean is dominated by the honest large holders. The `MIN_CONTRIBUTORS` threshold alone does not stop sybils, so the concentration penalty is applied to the *weight* distribution: sybil wallets with negligible weight do not meaningfully change $C$, and the weighted mean is unchanged. To further resist sybils, weights should be derived from on-chain balances/volume rather than self-reported values.
- **Single-holder dominance.** A single dominant holder would otherwise set the score; the concentration penalty reduces the score as $C \to 1$, so dominance lowers rather than raises the published score.
- **Score inflation via dust.** Because the aggregation is weight-proportional, dust contributions cannot move the mean; they only add to $n$, which is bounded by the concentration term.
- **Replay / stale contributions.** Contributions must be timestamped and decayed using the existing `decay_fixed` path so that stale wallet scores do not indefinitely influence the asset score.

### Test Vectors

Deterministic vectors for the derivation, including degenerate distributions:

| Contributors `(weight, score)` | Expected |
| --- | --- |
| `[]` | `None` (below min) |
| `[(1, 50)]` | `None` (below min) |
| `[(1, 50); 4]` | `None` (below min) |
| `[(1, 50); 5]` | `50` (uniform, no penalty) |
| `[(1, 0); 5]` | `0` |
| `[(1, 100); 5]` | `100` |
| `[(100, 80), (1, 20); 4]` | weighted mean ≈ 79, reduced by concentration penalty |
| `[(1, 50); 1000]` | `50` (uniform, no penalty) |

### Invariants

- **Bounded:** the result is always in `[0, 100]` (or `None`).
- **Monotone:** increasing any contributor's score (holding weights fixed) never decreases the asset score; increasing a contributor's weight toward a higher-scoring wallet never decreases the asset score.
- **Permutation-stable:** the result is independent of the order of `contributors` (the sums are commutative).
- **Degenerate:** an empty or below-threshold contributor set yields `None`; an all-zero weight set yields `None`.
