# Aggregator Conflict Resolution Policy

When scores are replicated across multiple shards, the aggregator must resolve
conflicts between potentially different values from each shard. The
`ConflictPolicy` enum controls which strategy is used.

Configuration conflicts are handled separately from score conflicts. Use
[`aggregator-split-brain.md`](aggregator-split-brain.md) and
`detect_split_brain(wallet, asset_pair)` to detect shard configuration drift
before relying on a fan-out read.

## Policy Variants

### `HighestScore` (default)

Pick the shard whose score (or aggregate score) is numerically highest.

- `get_score`: selects the `RiskScore` with the largest `score`
- `get_aggregate_score`: selects the `AggregateRiskScore` with the largest
  `aggregate_score`

This is the default policy (used if no policy has been explicitly set).

### `MostRecent`

Pick the shard whose timestamp is newest.

- `get_score`: selects the `RiskScore` with the largest `timestamp`
- `get_aggregate_score`: selects the `AggregateRiskScore` with the largest
  `last_updated`

Useful when score freshness matters more than magnitude (e.g. a rapidly changing
market).

### `Median`

Combine shard scores with the (lower) median instead of picking a single winner.

- `get_score`: sorts the shard `score` values ascending and returns the element
  at index `(n - 1) / 2` (integer division).
- `get_aggregate_score`: same rule applied to `aggregate_score`.

With an even number of shards the lower of the two middle values is chosen, so
the result never exceeds the true median. This is the safer (higher risk) side
when the two middle values disagree.

### `TrimmedMean`

Discard a governed fraction of the extreme values on each side, then average the
remainder.

- The trim fraction is set with `set_trim_fraction(Env, u32) -> Result<(), ScoreError>`
  and read with `get_trim_fraction(Env) -> u32`. It is expressed in basis points
  (1 bp = 0.01%), must be in `0..=2500` (0%–25%), and defaults to `0` (plain mean).
- For `n` shards, `k = (n * trim_bps) / 10_000` values are dropped from each
  end. If `2 * k >= n` the trim is clamped to `k = (n - 1) / 2` so at least one
  value survives.
- The remaining values are summed and divided by their count using integer
division, which truncates toward zero. Truncation rounds the combined score
*down*, i.e. toward the safer (higher risk) side.

### `MadRejection`

Median-absolute-deviation outlier rejection followed by a weighted mean.

1. Compute the median `m` of the shard scores (lower median, as above).
2. Compute the absolute deviations `|x_i - m|` and take their lower median `mad`.
3. Reject any shard whose deviation exceeds `mad * mad_multiplier_bps / 10_000`.
   The multiplier is set with `set_mad_multiplier(Env, u32) -> Result<(), ScoreError>`
   and read with `get_mad_multiplier(Env) -> u32`; it must be in `1..=100_000`
   (0.01x–10x) and defaults to `30000` (3x).
4. Average the surviving shards with the same integer division as `TrimmedMean`.

If every shard is rejected (e.g. `mad == 0` and all deviations are non-zero),
the policy falls back to the plain median `m` rather than returning an error.

## Degenerate Cases

Robust statistics degenerate with very few shards. The policies are defined as
follows so behavior is always deterministic:

- **One shard:** every policy returns that shard's value unchanged.
- **Two shards:** `Median` returns the lower value. `TrimmedMean` clamps the trim
  to `k = 0` and returns the truncated mean of both values. `MadRejection`
  rejects nothing unless the multiplier is `0`, and returns the truncated mean.
- **Three shards:** `Median` returns the middle value. `TrimmedMean` with a
  non-zero trim fraction drops the lowest and highest and returns the middle
  value. `MadRejection` rejects only values farther than the multiplier times the
  MAD from the median.

## Missing and Stale Shards

Shards that do not respond, or whose response is older than the staleness
threshold, are excluded before any policy is applied. If fewer than one shard
remains the call returns `ScoreError::NoShardsAvailable`. If exactly one shard
remains, the policy returns its value unchanged (see above). Staleness is
measured against the same threshold used by `MostRecent`.

## Determinism and Rounding

All policies are deterministic and integer-only:

- Sorting is stable and ties are broken by shard index, so equal scores never
  change the result across runs.
- Averages use integer division, which truncates toward zero. Truncation lowers
  the combined score, i.e. it favors the safer (higher risk) side when the exact
  value is ambiguous.
- Medians use the lower middle element for even counts, again favoring the safer
  side.

## Cost Analysis

Let `n` be the number of responding shards. Costs are given in relative units
(sort comparisons plus arithmetic), ignoring the fixed fan-out cost.

| Policy         | Time        | Extra storage        | Max shards |
| -------------- | ----------- | -------------------- | ---------- |
| `HighestScore` | O(n)        | none                 | unbounded  |
| `MostRecent`   | O(n)        | none                 | unbounded  |
| `Median`       | O(n log n)  | none                 | 64         |
| `TrimmedMean`  | O(n log n)  | trim fraction (u32)  | 64         |
| `MadRejection` | O(n log n)  | multiplier (u32)     | 32         |

The sort-based policies are capped to bound worst-case gas: `Median` and
`TrimmedMean` accept at most 64 shards, and `MadRejection` (which sorts twice)
accepts at most 32. Fan-out reads that exceed the cap for the active policy
return `ScoreError::TooManyShards`; callers should fall back to `HighestScore`
or reduce the shard set.

## Choosing a Policy

- Use `HighestScore` (default) when shards are trusted and you want the most
  conservative single reading.
- Use `MostRecent` when freshness dominates and stale values are the main risk.
- Use `Median` when a small number of shards may be faulty or manipulated and you
  want a bounded influence from any single shard.
- Use `TrimmedMean` when you can govern an expected outlier fraction and want a
  smooth combined value; keep the trim fraction at or below the fraction of
  shards you expect to be bad.
- Use `MadRejection` when outliers are rare but potentially large; keep the
  multiplier at 3x or higher so legitimate spread is not rejected.

Safe configurations:

- Keep the shard count at or below the cap for the active policy (64 for
  `Median`/`TrimmedMean`, 32 for `MadRejection`).
- Keep `trim_bps <= 2500` and `mad_multiplier_bps >= 30000`.
- Always configure a staleness threshold so stale shards are excluded before
  aggregation.

## API

### `set_conflict_resolution_policy(Env, ConflictPolicy) -> Result<(), ScoreError>`

Sets the active policy. Requires admin authorization. Returns
`ScoreError::NotInitialized` if the contract has not been initialized.

### `get_conflict_resolution_policy(Env) -> ConflictPolicy`

Returns the current policy, or `ConflictPolicy::HighestScore` if none has been
set.

### `set_trim_fraction(Env, u32) -> Result<(), ScoreError>`

Sets the trimmed-mean trim fraction in basis points. Requires admin
authorization. Returns `ScoreError::InvalidParameter` if the value exceeds
`2500`.

### `get_trim_fraction(Env) -> u32`

Returns the current trim fraction in basis points, or `0` if unset.

### `set_mad_multiplier(Env, u32) -> Result<(), ScoreError>`

Sets the MAD rejection multiplier in basis points. Requires admin
authorization. Returns `ScoreError::InvalidParameter` if the value is `0` or
exceeds `100_000`.

### `get_mad_multiplier(Env) -> u32`

Returns the current MAD multiplier in basis points, or `30000` if unset.

## Example

```rust
// Set to MostRecent
client.set_conflict_resolution_policy(&ConflictPolicy::MostRecent);

// get_score now returns the score from the shard with the newest timestamp
let score = client.get_score(&wallet, &pair);

// Switch to a robust policy
client.set_conflict_resolution_policy(&ConflictPolicy::TrimmedMean);
client.set_trim_fraction(&1000); // drop 10% from each end

client.set_conflict_resolution_policy(&ConflictPolicy::MadRejection);
client.set_mad_multiplier(&30000); // reject beyond 3x MAD

// Switch back to HighestScore
client.set_conflict_resolution_policy(&ConflictPolicy::HighestScore);
```
