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
