# Alignment: `ILedgerLensScore` ↔ On-Chain Risk Score Registry Interface (SEP draft)

**Status:** Draft · **SEP:** `docs/standards/sep-risk-score-registry-interface.md` v0.1.0 ·
**Interface under review:** `ILedgerLensScore` v3 (`docs/interface-spec.md`) ·
**Contract:** `contracts/ledgerlens-score` (`interface_version: 3`, `CONTRACT_VERSION: 5`)

This document is the answer to acceptance criterion 3 of
[#1248](https://github.com/Ledger-Lenz/Ledgerlens-contract/issues/1248): the
repository's interface specification is either aligned to the draft or diverges
from it **with reasons**. This file does that, clause by clause, and then records
the findings the comparison surfaced in the shipped contract and its
documentation.

**Headline:** `ledgerlens-score` satisfies every `must`-level conformance vector
in `tests/conformance/conformance_vectors.json` with **no change to its ABI,
storage layout, events, or error enum**. It deviates on one `should`-level vector
(`CORE-017`, the `risk` root capability) and on one `should`-level clause (9.3's
advisory no-events direction, §3 below). Neither deviation is a functional gap, and
neither required a code change to this repository.

---

## 1. Clause-by-clause mapping

| SEP clause | Requirement | `ILedgerLensScore` | Status |
|---|---|---|---|
| 1.1 | Read surface only; production and governance out of scope | §1 canonical functions are all reads | Aligned |
| 3.1 | `query_risk_gate -> bool` | §1.1, identical signature | Aligned |
| 3.1 | `get_score -> Result<RiskScore, Error>` | §1.5 "Direct read functions", `Err(ScoreNotFound)` when absent | Aligned |
| 3.1 | `supports_interface(Symbol) -> bool` | §1.3, identical signature | Aligned |
| 3.1 | `get_interface_metadata() -> InterfaceMetadata` | §1.4, identical struct | Aligned |
| 3.1 | `get_version() -> u32` | §3, plus `get_contract_version` as an alias | Aligned |
| 3.1 | `query_risk_gate_with_confidence -> bool` (SHOULD) | §1.2, identical signature | Aligned |
| 3.2 | `initialize` excluded from the portable surface | §1 does not specify `initialize`; contract takes `(admin, service)` | Aligned |
| 3.4 | Out-of-vocabulary `scope` fails closed, never traps | `asset_pair_is_bounded`, returns `false` (`MAX_ASSET_PAIR_BYTES = 9`) | Aligned |
| 4.1 | `RiskScore` = 4 named portable fields; extra fields allowed | All 4 present with identical names/types, interleaved with 6 provider-specific fields | Aligned (§2 explains why) |
| 4.2 | `InterfaceMetadata` layout | Identical 4-field struct | Aligned |
| 4.3 | Reserved error discriminants 2, 3, 4, 5, 6, 7, 25 | All present with identical values; enum defines 50 codes total | Aligned |
| 4.4 | `score` event: `(score, version, subject, scope)` + values | `events::score_submitted`, same topic order, `EVENT_VERSION: u32` | Aligned |
| 5.1 | No data ⇒ `false` | §1.1 "Conservative on the unknown" | Aligned |
| 5.2 | Strict `<` comparison | §1.1 "The comparison is strict" | Aligned |
| 5.3 | Inclusive confidence floor, `max()` of both floors | §1.2 "Effective confidence floor" | Aligned |
| 5.4 | `threshold > 100` ⇒ `false`, no trap, `threshold = 0` ⇒ `false` | §1.2 bullets 4–5; `u32::MAX` covered by `adversarial_consumer.rs` | Aligned |
| 5.5 | Deterministic gate | Proven by `tests/composability/tests/adversarial_consumer.rs` "Threat 1" | Aligned |
| 5.6 | Provider does not enforce freshness | §5 "Freshness is separate from confidence" | Aligned |
| 5.7 | Allow-listed callers get `false`, not a trap | `query_risk_gate*` return `false` when strict mode is on and the caller is absent from `get_gate_callers` | Aligned |
| 6.1 | `gate`, `meta` MUST; `cgate`, `score` SHOULD; no `caps` symbol | Advertises all four; does not advertise `risk` | **Diverges — §4** |
| 6.2 | `supports_interface` total, never traps; metadata ⊆ supports | Total ✓; metadata omits 6 implemented symbols | **Diverges on the SHOULD direction — §5** |
| 6.3 | Non-zero `interface_version`; branch on capabilities | Reports `interface_version: 3`; §3 "prefer `supports_interface`" | Aligned |
| 7.1 | Transport failure resolved by explicit policy, default deny | The repo documents the consumer's side; contract side is `try_*` result shape | Aligned |
| 7.2 | `ScoreNotFound`, never a zero-valued struct | `get_score` returns `Err(ScoreNotFound)` | Aligned |
| 7.3 | Paused / uninitialised ⇒ fail closed | `query_risk_gate*` return `false`; `get_score` returns `ContractPaused` / `NotInitialized` | Aligned |
| 8.1 | `0..=100`, non-zero `timestamp`, in-range reads | Enforced on write; `get_score` returns stored values | Aligned |
| 8.3 | Reject rather than clamp | `submit_score` rejects out-of-range input | Aligned |
| 9.1 | Gate reads write no durable state | No persistent or instance write; TTL extension limited to the temporary gate-read marker | Aligned |
| 9.2 | Temporary read marker allowed if non-authoritative | `set_gate_read_ledger` marker is not read by the gate's decision path and expires with the entry | Aligned (see §3) |
| 9.3 | Gate reads SHOULD NOT emit events | `check_service_silence` can emit one liveness alert from a read, at most once per silence condition | **Diverges in wording — §3** |
| 9.4 | Gate reads MUST NOT trap | §1.1 "Never panics" | Aligned |
| 10 | Consumer obligations | §6 recommended integration patterns, §5 security considerations | Aligned |
| 12.2 | Within a major version: names, layouts, codes, symbols, procedure stable | `docs/interface-versioning-policy.md` enforces exactly this | Aligned |

---

## 2. Alignment note: the `RiskScore` payload (SEP 4.1)

**What differs.** The SEP defines a four-field provider-neutral payload
(`score`, `confidence`, `timestamp`, `model_version`). The shipped struct has ten
fields:

```rust
pub struct RiskScore {
    pub score: u32,
    pub benford_flag: bool,
    pub ml_flag: bool,
    pub timestamp: u64,
    pub confidence: u32,
    pub model_version: u32,
    pub benford_score: u32,
    pub ml_score: u32,
    pub network_score: u32,
    pub commitment: Option<Bytes>,
}
```

**Why this is the right trade.** The four SEP fields are the ones a consumer can
interpret without provider-specific knowledge; the other six describe *how* a
particular vendor reached its number, which is exactly what a provider-neutral
standard must not require. The portable fields are already present, with stable
names and stable types, so a consumer written against the four-field payload
decodes this provider's ten-field payload unchanged.

**Why the SEP is worded the way it is.** Clause 4.1 requires the four portable
fields to keep their names and types, allows a provider to define additional
fields beside them, and requires a consumer to ignore fields it does not
recognise. Decoding is by field name, not by position: a Soroban `#[contracttype]`
struct is XDR-encoded as an `ScVal::Map` keyed by field name
(`ScMap::sorted_from`), so the declaration order of a `RiskScore` carries no wire
meaning, and an extra field in the map is simply not read.

That detail is what makes this alignment real rather than aspirational. An earlier
draft of 4.1 said a provider "MAY append fields after `model_version`" and that a
consumer "MUST decode the four portable fields by position". Under that wording
this struct would have **failed**: `benford_flag` and `ml_flag` sit between
`score` and `timestamp`, so no positional rule could describe it, and the only
options would have been to declare the shipped contract non-conformant or to
propose an ABI change. Requiring named fields instead of positional ones is
strictly weaker for consumers and it is the accurate description of the encoding.

**Not a change.** No field is added, removed, reordered, or retyped. Consumers
written against `docs/interface-spec.md` are unaffected.

**Related documentation defect found and fixed in this PR.** The `RiskScore`
listing in `docs/interface-spec.md` §2 showed six fields and omitted
`benford_score`, `ml_score`, `network_score`, and `commitment`, which have been in
the struct all along. The listing now matches `types.rs`. This was a
documentation-accuracy fix, not a behaviour change.

---

## 3. Divergence: read-side side effects (SEP 9.1, 9.2)

**What differs.** `docs/interface-spec.md` §1.1 describes `query_risk_gate` as
"**Side-effect free.** It is a pure read and does not even extend storage TTL —
calling it does not mutate LedgerLens state", and the metadata advertises the
`side_effect_free` semantic constraint. The implementation does two things a
"pure read" would not:

1. `storage::set_gate_read_ledger` writes the current ledger sequence to
   **temporary** storage per `(wallet, asset_pair)` and extends that entry's TTL.
   This is the flash-loan protection added for issue #300: it lets the contract
   distinguish "a caller has been polling this pair this block" from "a caller
   never checked", which the flash-protection path needs.
2. `check_service_silence` may emit a `ServiceSilenceAlertEvent` during a read,
   when the heartbeat threshold has been exceeded and no alert has been emitted
   yet. It is a once-per-condition alert, not a per-read event.

**How the SEP was written to accommodate this honestly.** Rather than declaring
the shipped behaviour non-conformant, clauses 9.1 and 9.2 were drafted to name the
actual invariant: a gate read MUST NOT write *durable* state — no persistent or
instance entry created, modified, or deleted, and no TTL extended for an entry that
exists independently of the read — while a bounded, per-`(subject, scope)`
temporary marker IS permitted provided it is not authoritative for any read result
and expires with the entry it describes. The marker is not consulted by the gate's
decision path, so that is a true statement about this provider, and the escape
hatch is narrow enough that it cannot become a back door for a stateful gate.

Clause 9.3 is correspondingly a SHOULD, and the conformance vector (`CORE-016`)
asserts an **event count** rather than the absence of events, precisely because a
provider may legitimately emit one liveness alert while serving a read.

**Wording corrected in this PR.** `docs/interface-spec.md` §1.1 now states the
guarantee precisely — no durable state mutation, TTL extension limited to the
temporary gate-read marker, and the liveness-event caveat — instead of claiming
pure-read semantics the code does not have. The `side_effect_free` constraint
symbol itself is unchanged, because it belongs to the deployed ABI and consumers
are told in SEP §4.2 not to branch on constraint symbols; what changed is that its
meaning is now written down at all. It previously had none: `side_effect_free`
appeared in `docs/interface-spec.md` §1.4 as a bare word in a list, which is
roughly how the overclaim survived review unnoticed.

**Residual risk, stated plainly.** A read that writes temporary state is still
state, and an integrator reasoning from the *old* wording would draw the wrong
conclusion about cost. That is why the wording fix is part of this PR rather than
deferred.

---

## 4. Divergence: the `risk` root capability (SEP 6.1) — conformance vector `CORE-017`

**What differs.** The SEP registers `risk` as a SHOULD-level root capability
meaning "this deployment implements this standard". `ledgerlens-score` does not
answer `true` for it. This is the only `should`-level vector the shipped contract
fails, and it is the deviation asserted explicitly in
`tests/conformance/tests/ledgerlens_score.rs`.

**Why it was not "fixed" in this PR.**

- The standard is a **draft**. Pinning a stable, deployed interface to a symbol
  that a draft might still rename or drop is the wrong order of operations.
- Adding a symbol to `supports_interface` is behaviourally additive and
  explicitly non-breaking under `docs/interface-versioning-policy.md`
  ("a capability symbol, once published, will not be removed or repurposed
  within the same interface major version" — publication, not absence, is the
  commitment), so this is a small, safe follow-up once the SEP is accepted.
- The functional cost of the omission is zero. A consumer that finds no `risk`
  symbol learns nothing it cannot learn from probing `cgate` and `score`.

**The escape hatch is closed on purpose.** The conformance suite asserts the
deviation set is *exactly* `["CORE-017"]`. When the symbol is added, that
assertion fails and the test is updated in the same change — so the deviation list
cannot silently grow and cannot silently shrink.

---

## 5. Divergence: metadata under-advertisement (SEP 6.2)

**What differs.** `supports_interface` answers `true` for 23 symbols, but
`get_interface_metadata().capabilities` lists 17. Six implemented capabilities are
absent from the metadata list:

`hpag`, `var`, `histogram`, `rgate`, `dprv`, `arch`

**Which direction of clause 6.2 is affected.** The MUST direction — "every symbol
in the metadata list MUST be answered `true` by `supports_interface`" — **holds**,
and vector `CORE-018` verifies it. The affected direction is the SHOULD: "a
provider SHOULD also list every symbol it answers `true` for in the metadata."

**Why the MUST direction is the one that is normative.** A consumer that reads
the metadata list and calls the function it names must not find a hole — that is
a runtime trap in someone else's protocol. Under-advertisement costs
discoverability, which is annoying. Only the first is unsafe.

**Why the suite cannot judge the SHOULD.** Detecting under-advertisement requires
enumerating a provider's own vocabulary, which is provider-specific by
definition; a provider-neutral vector file has no way to name `dprv`. The
requirement stays in the prose, and the concrete instance is recorded here.

**Action.** Follow-up issue, not this PR: either add the six symbols to the
metadata list or document why they are deliberately metadata-invisible. The
capability table in `docs/interface-spec.md` §1.3, which was missing the same six,
was corrected in this PR.

---

## 6. Aligned behaviour that is worth stating explicitly

These are the properties the conformance suite actually exercises, and where they
come from in the shipped contract:

| Property | Where it lives in `ledgerlens-score` | Vectors |
|---|---|---|
| Unknown subject ⇒ `false` | `peek_score` returns `None` ⇒ `false` | `CORE-001` |
| `score == threshold` ⇒ `false` | `risk.score < gate_threshold` | `CORE-004` |
| `threshold > 100` and `u32::MAX` ⇒ `false`, no trap | explicit range check | `CORE-007`, `CORE-008` |
| Confidence floor is inclusive | `risk.confidence >= effective_floor` | `EXT-003` |
| Provider-wide floor cannot be weakened | `max(min_confidence, global_min_confidence)` | `EXT-002` |
| Gate ignores staleness | no age term in the gate | `EXT-007` |
| No-data is an error, not a zero struct | `Err(ScoreNotFound)` | `CORE-014` |
| Published score round-trips in range | `get_score` | `CORE-015` |
| Gate read emits no event for a live provider | `check_service_silence` is a no-op below the heartbeat threshold | `CORE-016` |
| Discovery is total | `supports_interface` returns `false` for unknown and empty symbols | `CORE-012` |
| Non-zero interface version | `interface_version: 3` | `CORE-013` |

One behavioural note that is conformant but worth recording: `ledgerlens-score`
returns `false` from the gate for **embargoed** subjects and for subjects inside
the hysteresis risk band. Both are fail-closed resolutions of "no usable signal",
which is what clause 5.1 permits. A consumer that expects `true` for a subject it
believes is safe will see `false`; the reason is provider governance, not
interface semantics, and it belongs in a provider's documentation rather than in
the standard.

---

## 7. What was deliberately *not* standardised

| Left out | Why |
|---|---|
| Write path / `submit_score` shape | Producer authorisation is governance. A standard that fixed it would be a standard for one provider's trust model. |
| `initialize` signature | `(admin, service)` vs `(admin)` is a wiring choice. No consumer calls it. |
| Aggregation (`get_aggregate_score`, `aggr`) | The combination rule across providers is protocol-specific; a default adopted by inertia is worse than none. |
| Score history, attestation, consensus, delegation, freezing, reconciliation, verkle commitments | Provider-specific capability surfaces. A consumer reaches them through `supports_interface`, not through the standard. |
| Relative/percentile gating (`query_risk_gate_relative`) | Depends on a population histogram, which is a provider-specific data model. |
| Staleness policy | See SEP §5.6 and Design Rationale. |
| Privacy of scored subjects | Already covered for this provider by `docs/privacy-model.md`; a standard would have to generalise it, which is a different and larger document. |

---

## 8. Reproducing this alignment

```bash
# The shipped provider against the normative vectors.
cargo test -p conformance-tests --test ledgerlens_score

# The reference provider against the same vectors.
cargo test -p conformance-tests --test reference_provider

# The suite's own integrity checks and negative controls.
cargo test -p conformance-tests --test harness_self_check
```

The vector file is the normative artefact; the harness is one translation of it.
`tests/conformance/tests/ledgerlens_score.rs` asserts the exact deviation set, so
a change in the shipped contract's behaviour shows up as a failing test rather
than as a silently stale document.
