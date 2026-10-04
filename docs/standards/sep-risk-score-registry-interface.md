---
SEP: To Be Assigned
Title: On-Chain Risk Score Registry Interface
Author: LedgerLens maintainers
Status: Draft
Created: 2026-09-26
Updated: 2026-09-26
Version: 0.1.0
Discussion: https://github.com/Ledger-Lenz/Ledgerlens-contract/issues/1248
---

## Simple Summary

Define a minimal, provider-neutral read interface for on-chain risk scores, so a
protocol can ask any conforming registry "is this subject currently acceptable to
proceed?" without knowing or trusting the vendor that produced the number.

The interface is a small set of reads plus one discovery function:

- `query_risk_gate(subject, scope, gate_threshold) -> bool` — the integration
  primitive. Infallible, fail-closed, strictly below the threshold.
- `query_risk_gate_with_confidence(subject, scope, gate_threshold, min_confidence) -> bool`
  — the same, plus a confidence floor, for decisions where "I barely know" must
  not read as "safe".
- `get_score(subject, scope) -> Result<RiskScore, Error>` — the advisory payload,
  with `confidence`, `timestamp` and `model_version`.
- `supports_interface(capability) -> bool` — capability discovery, so a consumer
  can feature-detect instead of pinning a version.

The standard deliberately says nothing about *how* a score is produced. Models,
Benford analysis, machine learning, attestations, and governance are all out of
scope. That is what makes the interface provider-neutral, and it is why the
portable core is small enough to be boring.

## Dependencies

| Dependency | Kind | Notes |
|---|---|---|
| Stellar Soroban contracts | Platform | `soroban-sdk` 21.x; `Symbol`-keyed capabilities, `#[contracttype]` payloads, `#[contracterror]` codes |
| `docs/interface-spec.md` (`ILedgerLensScore`, interface version 3) | Normative source | The draft is a provider-neutral subset of this surface. Function names, capability symbols, data layout, and error discriminants are inherited unchanged. |
| `docs/interface-versioning-policy.md` | Normative source | Defines append-only capabilities, stable error discriminants, and what counts as breaking. |
| `contracts/reference-provider` | Reference implementation | Written for this draft; the executable form of every MUST. |
| `tests/conformance` (`conformance_vectors.json`) | Conformance suite | Provider-neutral vectors any language can execute. |
| RFC 2119 | Conventions | MUST / MUST NOT / SHOULD / SHOULD NOT / MAY as in RFC 2119. |

No dependency on any particular detection model, vendor, or chain extension.

## Motivation

A risk score is only useful to a protocol if the protocol can read it, and today
that means binding to one registry's ABI. Three costs follow from that binding:

1. **Vendor lock-in.** A consumer written against one registry's client cannot
   read another's, even when both publish the same four numbers. Switching
   providers is a rewrite of the integration and its tests, not a config change.
2. **Silent semantic drift.** A function called `query_risk_gate` may mean
   "score below threshold", "score below threshold *and* the caller is
   allow-listed", or "score below threshold unless the provider is paused". All
   three return `bool`, and only one of them is what a risk-conscious consumer
   means. Nothing in the type system distinguishes them, so the difference shows
   up as an incident.
3. **No shared notion of failure.** A trap, an error return, and "no data" are
   three different things. A consumer that collapses them — because the ABI makes
   collapsing them convenient — can read a dead provider as a safe subject. This
   is the failure mode with the highest expected cost and the least visible
   symptom.

Meanwhile the natural alternative, a shared SDK, has its own failure mode: it
turns a standard into a library release, ties every provider to one team's
release cadence, and does not help providers written in other languages.

This draft therefore fixes three things and leaves everything else to the
provider: a small read surface, precise semantics for the numbers, and a data-only
conformance suite that any implementation can run.

## Abstract

An **on-chain risk score registry** ("provider") stores one score per
`(subject, scope)` pair and answers reads about it. `scope` is a provider-defined
`Symbol` — an asset pair, a market, a jurisdiction, a use case — so a subject can
be scored differently for different contexts without changing the interface.

A **consumer** is any contract that guards an action on a score. The interface is
one-sided: consumers read, providers write. There is no write path in the
portable surface, because how a provider authorises its producers is a governance
decision, not an interface property.

The semantics that matter are:

- **The gate is fail-closed and infallible.** `query_risk_gate` returns `bool` and
  never traps. Every indeterminate condition — no data, paused provider,
  uninitialised provider, out-of-range threshold — resolves to `false`.
- **The comparison is strict.** `score < gate_threshold` passes. A score equal to
  the threshold does not.
- **Confidence is epistemic weight, not safety.** A score of `30` with a
  confidence of `5` is treated exactly like no data. A low-confidence score is
  never evidence of safety.
- **Freshness belongs to the consumer.** The provider reports the observation
  `timestamp` and does not decide staleness; the consumer applies its own bound
  and fails closed when exceeded. A provider that enforces a max-age policy
  internally is still conformant, but the consumer's bound is what the standard
  requires, because only the consumer knows how much delay its own risk tolerates.
- **Absence of an answer is not an answer.** A transport failure (trap, missing
  contract, undecodable response) MUST be distinguishable from a domain error,
  and a consumer MUST resolve it by explicit policy, defaulting to deny.

## Specification

### 1. Scope and conformance

1.1 This specification defines the **read** interface of a risk score registry and
the obligations of a consumer that reads it. It does not define how scores are
produced, who may produce them, or how a provider is governed.

1.2 A **provider** conforms when it implements every MUST in §3–§9 and passes the
conformance suite in `tests/conformance/conformance_vectors.json`. The suite is
normative; prose is not a substitute for it.

1.3 A **consumer** conforms when it follows §10. Consumer conformance is not
tested by the suite, because it is a property of the integrating protocol.

1.4 Anything not required by a MUST in this document is provider-specific. A
provider MAY expose more functions, more capabilities, more score fields, more
events, and a different `initialize` signature. Consumers MUST NOT depend on
anything provider-specific except through capability discovery.

1.5 The words MUST, MUST NOT, REQUIRED, SHALL, SHOULD, SHOULD NOT, RECOMMENDED,
and MAY are to be interpreted as in RFC 2119 when, and only when, they appear in
all capitals.

### 2. Terminology

| Term | Meaning |
|---|---|
| provider | A contract implementing this interface. Publishes scores, answers reads. |
| consumer | A contract (or off-chain actor) that consults a provider before permitting an action. |
| subject | The `Address` being scored. Usually an account; never assumed to be a wallet. |
| scope | The `Symbol` key a score is filed under. Provider-defined vocabulary. |
| score | `u32` in `0..=100`. Higher is riskier. |
| confidence | `u32` in `0..=100`. Confidence *in the score*, not probability of safety. |
| effective confidence floor | `max(min_confidence, global_min_confidence)`. |
| transport failure | The call did not complete: trap, missing contract, undecodable response. |
| domain error | The provider answered with an error value. The provider *did* answer. |
| no data | The provider answered that it holds no score for `(subject, scope)`. |

The distinction between **transport failure**, **domain error**, and **no data**
is load-bearing and is the subject of §7.

### 3. Interface surface

3.1 A provider MUST expose the following functions.

| Function | Requirement | Returns |
|---|---|---|
| `query_risk_gate(subject, scope, gate_threshold) -> bool` | MUST | `bool`, never `Result` |
| `get_score(subject, scope) -> Result<RiskScore, Error>` | SHOULD (see 3.3) | score payload or a domain error |
| `supports_interface(capability: Symbol) -> bool` | MUST | `bool` |
| `get_interface_metadata() -> InterfaceMetadata` | MUST | versioned discovery struct |
| `get_version() -> u32` | MUST | provider build version, diagnostics only |
| `query_risk_gate_with_confidence(subject, scope, gate_threshold, min_confidence) -> bool` | SHOULD | `bool` |
| `get_score_opt(subject, scope) -> Option<RiskScore>` | MAY | same payload, absence as `None` |

A provider MAY expose `get_contract_version` as an alias of `get_version`;
`ILedgerLensScore` does, and a consumer written against either spelling should
keep working.

3.2 `initialize` is **not** part of the portable surface. It exists on every
provider, its signature differs between providers (`ILedgerLensScore` takes
`(admin, service)`; the reference provider takes `(admin)`), and no consumer ever
calls it: a consumer integrates with an already-deployed provider address.

3.3 `get_score` is SHOULD rather than MUST because a minimal provider may expose
nothing but a gate. A provider that advertises the `score` capability MUST behave
as §8 requires; a provider that does not advertise it is not judged on §8. The
advisory nature of the payload is deliberate: the gate is the contract, the score
is a convenience that carries provider-specific meaning.

3.4 `scope` MAY be any `Symbol` the provider accepts. A provider MAY bound the
set of scopes it accepts and MUST resolve an out-of-vocabulary scope the same way
it resolves an unknown subject (fail closed, §5.1) rather than trapping.

### 4. Data layout

4.1 `RiskScore` — the portable payload.

```rust
#[contracttype]
pub struct RiskScore {
    pub score: u32,
    pub confidence: u32,
    pub timestamp: u64,
    pub model_version: u32,
}
```

Field names and types are the contract, and they MUST NOT change within a major
interface version: a provider MUST NOT rename, retype, or remove any of the four
portable fields.

A provider MAY define additional fields alongside the portable core — for
example a provider that exposes a per-signal breakdown of how a score was
reached. A consumer MUST ignore fields it does not recognise and MUST NOT treat
their presence as a protocol violation; symmetrically, a consumer MUST NOT require
a provider-specific field to exist.

Decoding is by field name, not by position. A Soroban `#[contracttype]` struct is
XDR-encoded as an `ScVal::Map` keyed by field name, so the relative order of the
declaration carries no wire meaning and a consumer decodes the four portable
fields by name. That is what makes the extension rule above safe in practice:
`ILedgerLensScore` already interleaves provider-specific fields (`benford_flag`,
`ml_flag`, and four sub-scores) between the portable ones, and a consumer written
against the four-field struct decodes its ten-field payload unchanged.

4.2 `InterfaceMetadata` — discovery.

```rust
#[contracttype]
pub struct InterfaceMetadata {
    pub interface_version: u32,
    pub contract_version: u32,
    pub capabilities: Vec<Symbol>,
    pub semantic_constraints: Vec<Symbol>,
}
```

`semantic_constraints` SHOULD include `fail_closed` when the provider resolves
indeterminate conditions to "not safe to proceed", and MAY include
`side_effect_free` and `bounded_score_range`. Consumers MUST NOT branch on
constraint symbols: they are diagnostics.

4.3 Error conventions. A provider's error type is a `#[contracterror]` enum whose
discriminants are stable and append-only. The portable core reserves the
following codes, taken unchanged from `ILedgerLensScore` so that a consumer can
map them to the same branch regardless of provider.

| Code | Variant | Portable meaning |
|---:|---|---|
| 1 | `AlreadyInitialized` | one-time setup was attempted twice |
| 2 | `NotInitialized` | provider has not been initialised |
| 3 | `Unauthorized` | caller is not an authorised producer/admin |
| 4 | `InvalidScore` | `score` outside `0..=100` |
| 5 | `InvalidConfidence` | `confidence` outside `0..=100` |
| 6 | `ScoreNotFound` | no score for this `(subject, scope)` |
| 7 | `ContractPaused` | provider-wide circuit breaker active |
| 25 | `InvalidTimestamp` | `timestamp` is zero or otherwise not a valid observation time |

A provider MAY define additional codes with higher discriminants. A consumer MUST
treat an unrecognised code as "no usable answer" and resolve it by its own policy;
it MUST NOT assume the set is exhaustive.

4.4 Events. A provider MUST emit, on every accepted write, an event whose first
topic is `score` and whose second topic is a schema version (`u32`), followed by
the `subject` and `scope`. The event carries the published values. Consumers MUST
NOT treat events as part of the read interface, and MUST NOT rely on a write
event implying a subsequent read will observe it in the same transaction.
Providers MAY emit additional events (liveness alerts, failover notices); those are
out of scope.

The versioned-topic convention matters: a consumer that decodes events across
provider upgrades needs a place to branch on schema, and topic 1 is where every
provider in this ecosystem already puts it.

### 5. Gate semantics

5.1 **No data is not safe.** If the provider holds no score for
`(subject, scope)`, `query_risk_gate` MUST return `false`. A provider MUST NOT
return `true` for a subject it has never scored. This is the single most
important clause in the document: it is what makes "unknown" resolve the same way
in every provider.

5.2 **The threshold comparison is strict.** `query_risk_gate` returns `true` if
and only if a score exists and `score < gate_threshold`. A score equal to the
threshold MUST NOT pass. Concretely: with `gate_threshold = 75`, `score = 74`
passes and `score = 75` does not. Higher is riskier; a consumer that wants
"risky or worse is blocked" picks the threshold accordingly and does not need to
know how the provider rounds.

5.3 **Confidence is a floor, not a tiebreak.** Where
`query_risk_gate_with_confidence` is available, it MUST return `true` if and only
if all of the following hold:

1. a score exists for `(subject, scope)`;
2. `score < gate_threshold`;
3. `confidence >= max(min_confidence, global_min_confidence)`.

The floor is **inclusive**: `confidence == min_confidence` passes. A provider MUST
NOT use a provider-wide floor to *weaken* a caller's floor, nor a caller's floor to
weaken the provider-wide one; the stricter of the two always applies. A score below
the effective floor MUST be treated exactly like no data (5.1) — a provider MUST
NOT return `true` on the strength of a low score whose confidence is below the
floor.

5.4 **Out-of-range input fails closed and never traps.** If `gate_threshold > 100`
or `min_confidence > 100`, the gate MUST return `false` for every subject, because
no conformant score can be below a threshold above the scale. `gate_threshold = 0`
MUST return `false` for every subject. The gate MUST NOT panic for any `u32`
input, including `u32::MAX`. This is a denial-of-service requirement in both
directions: a consumer must be able to pass an unvalidated configuration value
without trapping its own transaction.

5.5 **The gate is deterministic.** Two calls to `query_risk_gate` with identical
arguments, in the same ledger, MUST return the same answer. A provider MUST NOT
return `true` for one call and `false` for the next without a state change or a
ledger change that could justify it.

5.6 **The gate does not enforce freshness.** The provider MUST NOT reject a
subject solely because its score is old: staleness is a consumer policy, and only
the consumer knows the detection lag its own risk posture tolerates. A provider MAY
apply a max-age policy of its own, and a consumer MUST NOT rely on one being
present. A provider MUST report the observation `timestamp` accurately (§4.1) so
the consumer can compute `now - timestamp` itself.

5.7 The gate MUST NOT be affected by a per-caller allowlist in a way that is
invisible to the consumer. A provider MAY restrict which addresses may read
(`ILedgerLensScore` does this via `set_gate_callers` plus
`set_gate_enforcement_mode`), and a restricted provider MUST return `false` for
non-allow-listed callers rather than trapping or erroring. The consumer-visible
consequence is documented in `docs/standards/ledgerlens-alignment.md`: a consumer
deployed with a registry it is not allow-listed for sees every subject as unsafe,
which is safe but not useful, and the fix is configuration rather than code.

### 6. Capability discovery and versioning

6.1 **Discovery is a function, and the function is mandatory.** `supports_interface`
MUST exist and MUST return `true` for every capability the provider implements.
The registered symbols are:

| Symbol | Requirement | Backing functionality |
|---|---|---|
| `gate` | MUST | `query_risk_gate` |
| `meta` | MUST | `get_interface_metadata` |
| `cgate` | SHOULD | `query_risk_gate_with_confidence` |
| `score` | SHOULD | `get_score` |
| `risk` | SHOULD | root: this deployment implements this standard |

There is deliberately no `caps` capability. A caller learns that discovery exists
by calling it, and a provider that had to declare the availability of
`supports_interface` itself would gain nothing but a way to fail the requirement.
A provider MAY register additional symbols; they are provider-specific and
consumers MUST treat an unrecognised symbol as "unknown capability", which
`supports_interface` MUST answer `false` rather than trap.

6.2 **Discovery MUST be total and consistent.** `supports_interface` MUST NOT trap
for any `Symbol`, including the empty symbol and symbols longer than nine
characters. It MUST return `false` for any symbol the provider does not
implement. In addition, every symbol listed in `get_interface_metadata().capabilities`
MUST be answered `true` by `supports_interface`: a consumer that reads the
capability list and then calls the function it names MUST NOT find a hole.
A provider SHOULD also list every symbol it answers `true` for in the metadata,
and SHOULD declare the symbols that matter to a non-obvious reader — a consumer
that has to try six symbols to discover the confidence gate will not try the
seventh.

6.3 **Versions are diagnostics.** `get_interface_metadata` MUST report a non-zero
`interface_version` and a `contract_version`. `interface_version` is the
provider's own interface version and need not equal this document's `Version`: a
provider may track several interface specifications at once, and mapping between
them is the provider's business. Consumers MUST branch on `supports_interface`,
never on a version comparison — not against the provider's number and not against
this document's: a newer deployment that adds capabilities still answers `true`
for the ones a consumer depends on, whereas a hardcoded version check breaks on
an additive upgrade. A capability symbol, once published, MUST NOT be removed or
repurposed within a major interface version.

6.4 The consumer-facing meaning of a provider is therefore the triple
`{capabilities, semantic_constraints, error codes}`, and the conformance suite
checks exactly that triple's observable consequences.

### 7. Failure modes

7.1 **A transport failure is not an answer.** If a cross-contract call to a
provider traps, targets an address with no contract, or returns something
undecodable, the consumer has learned *nothing* about the subject. It MUST NOT
interpret that as low risk. It MUST resolve the condition by an explicit,
documented policy, and the default MUST be to deny. A consumer that chooses to
allow on transport failure MUST make that choice visible in its own configuration
and MUST NOT inherit it implicitly from a provider. In a Soroban client this is
the distinction between the outer and inner `Result` of a `try_`-prefixed call; a
consumer that writes `try_query_risk_gate(..).unwrap_or(Ok(true))` has chosen
fail-open, whatever the comment says.

7.2 **No data is a domain error, not a transport failure.** `get_score` MUST
report the absence of a score as `Err(ScoreNotFound)`, not as an empty struct, a
zero-valued struct, or a trap. A zero-valued struct is the dangerous encoding: it
is indistinguishable from "score 0, maximum confidence", i.e. from the safest
possible subject. Consumers MUST treat `ScoreNotFound` as fail-closed unless they
have deliberately chosen otherwise.

7.3 **Other domain errors are the provider's answer, not a failure.** `NotInitialized`,
`ContractPaused`, `Unauthorized` and any unrecognised code mean the provider
declined to answer. A paused provider is a provider that has lost its data feed;
serving the last score it happened to receive as if it were current is the failure
mode the circuit breaker exists to prevent. Consumers MUST treat these as
fail-closed.

7.4 A provider MUST resolve every indeterminate condition available to it to
`false` or to a domain error. It MUST NOT return `true` because a lookup failed,
and it MUST NOT convert a domain error into a success value.

### 8. The score payload

8.1 `get_score` MUST return the published `RiskScore` with `score` and
`confidence` in `0..=100`, a `timestamp` equal to the observation time, and a
`model_version` the provider defines. `timestamp` MUST NOT be zero. Consumers MUST
treat the payload as advisory: it is provider-specific by construction, and only
the gate carries portable semantics.

8.2 `model_version` is opaque. A consumer MAY use it to notice that a provider
re-based its model — which invalidates historical comparisons — and MUST NOT
attempt to interpret its numbering.

8.3 Out-of-range writes MUST be rejected rather than clamped. A clamped `100` is
indistinguishable from a genuine maximum-risk observation, and the consumer cannot
tell "maximum risk" from "garbage input". A provider MUST NOT return a
`score`/`confidence` outside `0..=100` from `get_score`.

### 9. Read invariants

9.1 Gate reads MUST NOT write durable state. In particular they MUST NOT create,
modify, or delete persistent or instance entries, and MUST NOT extend the TTL of
any entry that exists independently of the read itself. A read that writes is a
read an attacker can use: a gate that pokes at durable state on every call can be
driven by anyone who can call the consumer, and a gate that extends TTLs changes
rent obligations nobody asked it to change.

9.2 A provider MAY maintain a bounded, per-`(subject, scope)` read marker in
temporary storage for anti-abuse purposes, provided the marker is **not
authoritative for any read result** and expires with the entry it describes.
`ILedgerLensScore` keeps such a marker (a gate-read ledger sequence used by its
flash-loan protection) and is conformant under this clause; see
`docs/standards/ledgerlens-alignment.md` §3.

9.3 Gate reads SHOULD NOT emit events. An event emitted from a read path makes
event-stream consumers unable to distinguish "the provider published something"
from "someone asked a question", which is exactly the distinction a monitoring
system needs. A provider that emits a liveness alert from a read path MUST emit it
at most once per alert condition, and the reference suite checks the count rather
than the presence of events, because a provider may legitimately emit a liveness
alert while a read is served.

9.4 Gate reads MUST NOT be able to trap. See 5.4. A consumer MAY therefore call
the gate with configuration values it has not validated, without risking a trap in
its own transaction.

### 10. Consumer obligations

10.1 A consumer MUST define, explicitly and in its own configuration, four
parameters: `gate_threshold`, `min_confidence`, `max_age_secs`, and its
`on_unavailable` policy. None of them is a protocol constant to be hardcoded
without thought: the right threshold for a 10 XLM swap and the right threshold for
a 1M XLM borrow are different numbers.

10.2 **The decision procedure** is normative, in this order:

1. If the provider advertises `cgate`, call `query_risk_gate_with_confidence`;
   otherwise call `query_risk_gate`.
2. If the call did not complete (transport failure, §7.1), apply the
   `on_unavailable` policy. Default: deny.
3. If the gate returned `false`, deny. Classify the reason for observability —
   no data, low confidence, above threshold — by reading the payload when the
   provider advertises `score`. Mis-attribution is an observability defect, not a
   safety defect: a denial with an unknown cause is still a denial.
4. If the gate returned `true` and `max_age_secs` is non-zero, read the payload,
   compute `age = now - timestamp`, and deny if `age > max_age_secs`.

10.3 **Freshness is enforced by the consumer** (§5.6). A gate that returned `true`
for a score published a year ago has told the consumer nothing about the present.
The consumer MUST fail closed on staleness, and MUST treat a stale score exactly
as it treats no data.

10.4 The default `on_unavailable` policy MUST be deny. A consumer that allows on
transport failure MUST record the choice in its own configuration and documentation
so that it is auditable; the reason to allow is availability pressure (a
provider outage must not halt a protocol), and the reason to record it is that this
is a risk decision, not an implementation detail.

10.5 A consumer MUST feature-detect with `supports_interface` before calling an
optional function, so that an older or leaner provider degrades instead of
trapping. A consumer MUST NOT cache a safe verdict beyond `max_age_secs`.

10.6 A consumer MUST NOT interpret a low confidence as a high risk *score*, nor a
missing score as a low score. The three states — safe, unsafe, unknown — are
distinct, and the interface is designed so that all three are expressible.

### 11. Conformance

11.1 `tests/conformance/conformance_vectors.json` is the normative conformance
artefact. Vectors are provider-neutral data; a provider in any language
translates them. The Rust harness in `tests/conformance` is one such translation.

11.2 Each vector is `must` or `should`. A `must` failure means the provider is not
conformant. A `should` failure is a reported deviation and does not break
conformance, so that a `should` can be aspirational without being unenforceable.

11.3 Vectors are gated on `requires_capability`, so a provider is only judged on
behaviour it claims to implement: a provider without `cgate` is not judged on the
confidence-gate vectors, and a provider without `score` is not judged on the
vectors that read the payload.

11.4 A provider claims conformance by running the suite against its own deployment
and reporting the result. Vector identifiers are stable and are never renumbered
or reused; a corrected vector keeps its id and gains a changelog entry.

11.5 The suite includes vectors for the failure modes that matter most: an
unknown subject, a score equal to the threshold, an unreachable provider, a
confidence floor, an out-of-range threshold, and a stale score. A suite that
cannot fail is worse than no suite, so the harness also carries negative
controls — a deliberately non-conformant provider — that prove the vectors detect
fail-open behaviour.

### 12. Versioning of this standard

12.1 This document carries a `Version` in its preamble, independent of any
provider's `interface_version`.

12.2 Within a major version, the following MUST NOT change: function names and
signatures in §3.1, the `RiskScore` and `InterfaceMetadata` layouts in §4, the
reserved error discriminants in §4.3, the registered capability symbols in §6.1,
and the decision procedure in §10.2.

12.3 A new major version may add functions, capabilities, and error codes. It
MUST NOT redefine an existing symbol or a reserved discriminant: a consumer
written against version N must keep working against version N+1 for everything it
used.

12.4 `Docs`/discussion happen in the open. Substantive changes are proposed as an
issue with a design note before a pull request, mirroring the process this draft
went through.

## Design Rationale

**Why a `bool` gate is the primitive.** A consumer's question is a yes/no question
about whether to proceed. Making the primitive return the raw score and leaving
each consumer to write the comparison guarantees N subtly different comparisons in
N consumers. Putting the comparison in the provider means the semantics are stated
once, tested once, and identical everywhere — which is the entire point of a
standard. The cost is that the provider is trusted with the comparison; §10.3 and
§5.6 keep the consumer in control of the parts it cannot delegate.

**Why fail-closed is a MUST and not a default.** Because `query_risk_gate` returns
`bool`, "no data" must collapse to a single value, and the two candidates are not
symmetric: collapsing to `true` is a silent authorisation bypass for every subject
the provider has not seen, while collapsing to `false` is a denial of service for
them. The standard picks the failure that is loud rather than the failure that is
profitable.

**Why confidence is a floor and not a factor.** Weighting a decision by
confidence invites each consumer to invent its own weighting, and every invented
weighting is a place for an attacker to find slack. A floor is a single, testable
predicate with one boundary condition, and it is falsifiable in both directions: a
score below the floor is denied, a score at the floor is allowed.

**Why freshness is the consumer's job.** The provider knows when it observed
something; the consumer knows how much delay its risk posture tolerates. A
provider-enforced max-age would either be too strict for a consumer that tolerates
delay, or be ignored by a consumer that needs less. Worse, a provider-enforced
max-age is invisible: a consumer that believes the gate means "currently
acceptable" would be wrong, and the interface would be lying by omission.

**Why `get_score` is SHOULD.** A gate-only provider is a legitimate provider: some
risk signals are genuinely only meaningfully reducible to a decision. Requiring
the payload would force such a provider to invent fields it has no honest answer
for, and invented fields are worse than absent ones. The reference consumer treats
`score` as optional and degrades: no payload means no freshness check and no denial
classification, both of which the consumer records as reduced assurance rather than
as full safety.

**Why capability discovery is a function and not a `caps` symbol.** The first draft
of this document required a `caps` capability meaning "discovery is available". It
was dropped: it is self-referential, it tells a caller nothing they cannot learn by
calling the function, and it would have made the shipped `ILedgerLensScore`
provider non-conformant for the omission of a symbol with no informational
content. A standard should not be able to fail on that.

**Why data-only vectors instead of a shared SDK.** Providers and consumers are
routinely written by different teams, in different languages, on different release
schedules. A suite that requires adopting a Rust crate to claim conformance fails
for a provider written in JavaScript, and it makes every conformance claim
contingent on a version number of somebody else's library. A JSON file of vectors
is the whole contract: adding a vector extends every provider's obligations
automatically, and no provider has to take a dependency to be tested.

**Why a reference provider in this repository.** A specification with no
implementation is a wish. The reference provider is deliberately boring — no
models, no attestation, no multisig, no aggregation — so that the floor is
visible, and it is written against the *existing* `ILedgerLensScore` names so that
the claim "a consumer written against one works against the other" is testable
rather than aspirational.

**Why `risk` is SHOULD, not MUST.** It is the one deviation the shipped provider
would take. Requiring it would force a breaking change to a stable, deployed
interface for zero functional gain: a consumer that finds no `risk` symbol learns
nothing it cannot learn from the absence of `cgate` and `score`. It is
recommended, and its absence is recorded as a deviation in
`docs/standards/ledgerlens-alignment.md`.

## Security Considerations

### Fail-closed defaults

- The gate MUST return `false` for unknown subjects (5.1), and the consumer's
  default `on_unavailable` policy MUST be deny (10.4). Together these mean no
  configuration error can silently produce an "allow".
- Out-of-range thresholds fail closed (5.4), so a consumer that passes an
  unvalidated configuration value cannot accidentally allow everything.
- Staleness fails closed (10.3), and a stale score is treated as no data rather
  than as the last known good state.
- A provider MUST NOT clamp invalid input (8.3), because a clamped value is
  indistinguishable from a real observation.

### Untrusted providers

A consumer that reads a score from a provider it does not control is trusting that
provider on three axes, and the interface narrows but does not eliminate each:

- **Honesty about the number.** The interface cannot verify a score. Mitigation:
  `model_version` lets a consumer notice re-basing, and multiple providers can be
  consulted and required to agree (§10.1 permits, and production deployments
  should use, more than one).
- **Availability and liveness.** A provider that stops publishing serves the last
  score it received. Mitigation: the consumer's freshness bound (10.3) converts a
  silent provider into a denial; a provider SHOULD also expose a circuit breaker
  whose activation is observable.
- **Targeted denial of service.** An attacker who can make a provider score them as
  risky is blocked, which is the intended behaviour; an attacker who can make a
  provider *stop answering* gets whatever the consumer's `on_unavailable` policy
  says. Mitigation: keep the default deny, and if availability pressure forces a
  fail-open policy, bound it — cap the number of consecutive failures tolerated
  before the integration is disabled outright, and alert on the policy firing.

The interface deliberately keeps trust delegation out of scope beyond one point:
§5.7 requires a restricted provider to fail closed rather than trap, so a consumer
that is not allow-listed sees `false` instead of an exception. A consumer that
needs delegated authority (a protocol acting on behalf of a user, an aggregator
querying on behalf of several protocols) MUST obtain it through the provider's own
governance, and MUST NOT infer it from a successful read.

### Trust delegation

- **The producer side.** How a provider decides who may write is out of scope
  (§1.1). A consumer MUST NOT assume that a score exists because a provider
  accepted it: providers are free to accept scores from anyone, from a multisig,
  or from an attested quorum, and those are materially different trust postures.
  A consumer that needs a specific posture MUST check the provider's governance
  documentation, or require a capability that only providers with that posture
  advertise.
- **The forwarding side.** A provider MAY answer for a subject it does not hold a
  score for, by delegating to a custodian (`ILedgerLensScore` supports score
  delegation). A consumer MUST treat a delegated answer as a normal answer — the
  interface gives no way to tell — and a provider MUST document the delegation
  rules it applies.
- **The scope side.** `scope` is provider-defined. A consumer MUST NOT assume that
  a score filed under one scope implies anything about another, and MUST NOT
  substitute a score from a different scope for a missing one.
- **Aggregation across providers.** A consumer combining several providers MUST
  define the combination rule itself (any, all, quorum, weighted). The standard
  does not define it, because the right rule is protocol-specific, and a
  standard-defined default would be adopted by inertia.

### Known limitations

- A conformant provider can be sybil-controlled, and nothing in the interface
  detects that. Mitigations are governance-level and belong to the deployment.
- A conformant provider can be accurate and wrong. Scores are model output; the
  interface standardises transport, not epistemology.
- The interface does not cover write-side integrity, proof of observation, or
  privacy of the subjects being scored. `docs/privacy-model.md` covers the last
  of these for `ledgerlens-score` specifically.

## Changelog

### 0.1.0 — 2026-09-26

- Initial draft, written for issue #1248.
- Defined the portable read surface: `query_risk_gate`,
  `query_risk_gate_with_confidence`, `get_score`, `supports_interface`,
  `get_interface_metadata`, `get_version`.
- Defined semantics for score, confidence, staleness, and the three failure
  classes (transport failure, domain error, no data).
- Reserved the portable error discriminants and the `score` event shape, both
  inherited unchanged from `ILedgerLensScore`.
- Registered the capability symbols `gate`, `meta`, `cgate`, `score`, `risk`.
  Dropped the `caps` symbol proposed in the first internal draft.
- Added the conformance suite: 33 provider-neutral vectors in
  `tests/conformance/conformance_vectors.json`, with negative controls in the
  harness.
- Added the reference provider `contracts/reference-provider` and its conformance
  run, plus a conformance run against the shipped `ledgerlens-score`.
- Recorded alignment with `docs/interface-spec.md` and every divergence with a
  rationale in `docs/standards/ledgerlens-alignment.md`.
- No change to the ABI, storage layout, events, or error enum of
  `contracts/ledgerlens-score`.
