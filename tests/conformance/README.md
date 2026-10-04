# Risk score registry conformance suite

Executable form of
[`docs/standards/sep-risk-score-registry-interface.md`](../../docs/standards/sep-risk-score-registry-interface.md)
§11.

**The normative artefact is [`conformance_vectors.json`](conformance_vectors.json),
not this Rust code.** If you implement the interface in JavaScript, Python, or
Rust with a different SDK, translate the JSON. You do not need to adopt anything
from this repository, and you do not need to match a version number of ours.

## What is in here

```
conformance_vectors.json   normative vectors: 33 cases, 31 must-level, 2 should-level
src/lib.rs                 the harness: vector execution, report rendering, and the
                           normative consumer decision procedure from SEP §10.2
tests/common/              one adapter per provider, implementing `RiskProvider`
tests/reference_provider.rs     conformance run for contracts/reference-provider
tests/ledgerlens_score.rs       conformance run for contracts/ledgerlens-score
tests/harness_self_check.rs     vector-file integrity + negative controls
```

## Running it against a provider in this repository

```bash
cargo test -p conformance-tests --test reference_provider
cargo test -p conformance-tests --test ledgerlens_score
cargo test -p conformance-tests --test harness_self_check
```

A `must` failure means the provider is not conformant. A `should` failure is
reported and does not break conformance.

## Running it against your own provider

Implement `RiskProvider` — five reads plus setup helpers — and add a test that
asserts `report.is_conformant()`:

```rust
impl RiskProvider for MyProvider {
    fn provider_id(&self) -> &str { "my-provider" }
    fn reset(&mut self) -> Address { /* fresh deployment, return an unscored subject */ }
    fn seed_score(&mut self, score: u32, confidence: u32, age_secs: u64) -> Address { /* … */ }
    fn now(&self) -> u64 { /* current ledger timestamp */ }
    fn make_unavailable(&mut self) { /* point the adapter at a contract-less address */ }
    fn make_available(&mut self) { /* … */ }
    fn gate(&mut self, subject: &Address, threshold: u32) -> ProbeResult<bool> { /* … */ }
    fn gate_with_confidence(&mut self, s: &Address, t: u32, c: u32) -> ProbeResult<bool> { /* … */ }
    fn score(&mut self, subject: &Address) -> ProbeResult<ScoreView> { /* … */ }
    fn supports(&mut self, capability: &str) -> ProbeResult<bool> { /* … */ }
    fn metadata_capabilities(&mut self) -> ProbeResult<Vec<String>> { /* … */ }
    fn interface_version(&mut self) -> ProbeResult<u32> { /* … */ }
    fn event_count(&mut self) -> usize { /* events emitted so far */ }
}
```

Two rules for the adapter, and they are the whole reason the suite is worth
running:

1. **Preserve the three-way distinction in `ProbeError`.** `Unavailable` means the
   provider did not answer (trap, no contract, undecodable response).
   `NotFound` means it answered "no data". `Rejected` means it answered with some
   other domain error. Collapsing these is the single most common way to
   accidentally fail this suite — and the single most dangerous way to fail in
   production, because a dead provider starts looking like a safe subject.
2. **Publish scores at `BASE_TIMESTAMP` and advance the clock by `age_secs`.**
   The suite asserts `timestamp == 1700000000` and computes staleness relative to
   it, so a provider that stamps its own "now" cannot pass.

## Writing the adapter for a language that is not Rust

Read the vector file. Each entry states:

| Field | Meaning |
|---|---|
| `id` | Stable identifier. Never renumbered or reused; a corrected vector keeps its id. |
| `level` | `must` (conformance gate) or `should` (reported deviation). |
| `clause` | The specification clause being enforced. |
| `probe` | Which read to exercise (see `Probe` in `src/lib.rs`). |
| `requires_capability` | Only run this vector if the provider advertises that symbol. |
| `setup` | Precondition: `unknown_subject`, `scored {score, confidence, age_secs}`, or `unavailable`. |
| `args` | Threshold, confidence floor, freshness bound, unavailable policy. |
| `expect` | Expected result: `bool`, `score`, `error`, `events`, `version`, or `decision`. |

The `decision` expectation is the most valuable one for a provider: it runs the
SEP §10.2 consumer decision procedure against your provider and checks the
verdict, so it tests what a consumer will actually do with you rather than what
your function returns in isolation.

## Why the harness is not the contract

A shared SDK would make every provider's conformance contingent on this
repository's release cadence, and would exclude providers written in other
languages. Adding a vector here extends every provider's obligations
automatically, with no dependency and no version negotiation.
