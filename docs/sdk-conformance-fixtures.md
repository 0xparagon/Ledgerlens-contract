# SDK Conformance Fixtures

LedgerLens SDKs for Rust, TypeScript, and Python must agree on the observable
contract in `tests/composability/sdk_conformance_fixtures.json`.

This repository actively enforces this fixture against the deployed `ledgerlens-score` contract via the `tests/composability/tests/sdk_conformance.rs` test suite. The test runs on every PR as part of the `cargo test --workspace` flow to prevent behavioral drift.

When a fixture case needs to change (e.g., adding a new error code or boundary condition):
1. Update `tests/composability/sdk_conformance_fixtures.json` in this repository.
2. Ensure the Rust conformance test still passes (`cargo test -p composability-tests --test sdk_conformance`), adjusting the contract or test harness if necessary.
3. Once the updated fixture is merged here, downstream SDK repositories (TypeScript/Python) can pull the new JSON and update their own runners.

## Contract

The fixtures exercise production-shaped consumers, not direct happy-path calls
into LedgerLens. Each consumer validates amount first, requires admin
authorization for oracle/configuration rotation, checks optional contract
version compatibility, rejects stale scores, then asks the LedgerLens gate.
Read-only decisions must not create persistent writes.

Transport failure is never reported as low risk. Integrators must configure one
of two explicit policies:

| Policy | Unavailable oracle behavior |
| --- | --- |
| `fail_closed` | Reject with `OracleUnavailable`. |
| `fail_open` | Allow only because configuration explicitly chose availability over risk freshness. |

Risk rejections remain distinct from operational failures:

| Fixture outcome | Meaning |
| --- | --- |
| `allow` | Score exists, is fresh, below threshold, and meets confidence/version requirements. |
| `reject_high_risk` | Score is missing, embargoed, equal/above threshold, or the gate returned false. |
| `reject_low_confidence` | Score is below threshold but below the consumer confidence floor. |
| `reject_stale` | Score age exceeds `max_staleness_secs` or LedgerLens reports it stale. |
| `oracle_unavailable` | Cross-contract call trapped, target is missing, or response cannot be decoded. |
| `unsupported_version` | Oracle version is lower than the configured required version. |

## Reason-Code Bitmap

Scores carry a fixed-width `reason_bits` bitmap alongside the existing score
fields so consumers can explain a decision without free-text reasons on-chain.
The bitmap is additive: it is carried as an optional submission field and
surfaced through a sibling read path, so the `RiskScore` struct layout and the
ABI of existing readers are unchanged. Older consumers that do not know about
`reason_bits` continue to read the existing fields and simply ignore the new
value.

Bit assignments are versioned in the reason-code registry. The current version
(`REASON_REGISTRY_VERSION = 1`) defines:

| Bit | Name | Meaning |
| --- | --- | --- |
| 0 | `WASH_CYCLE` | Repeated self-dealing / circular flow detected. |
| 1 | `VOLUME_SPIKE` | Volume deviates sharply from the wallet's baseline. |
| 2 | `BENFORD_DEVIATION` | Leading-digit distribution deviates from Benford's law. |
| 3 | `COUNTERPARTY_CONCENTRATION` | Activity concentrated in too few counterparties. |

Bits 4 and above are reserved. Unknown bits MUST be preserved verbatim on
round-trip and ignored safely: a consumer that does not recognize a bit must not
fail, reject, or reinterpret the score because of it. Consumers test individual
reasons with the helper `has_reason(reason_bits, bit)` (bitwise AND against the
single-bit mask) rather than comparing the whole bitmap.

Privacy: reason codes are coarse, fixed-width categories shared across many
wallets. They never encode wallet identity, amounts, counterparties, or timing,
so they add no wallet-identifying inference beyond the score itself. This is
consistent with `docs/privacy-model.md`; the bitmap is a classification of the
score, not a fingerprint of the wallet.

Conformance fixtures include `reason_bits` on every score case, including a
case with unknown/reserved bits set, and a compatibility case that reads new
data using the previous type definition (which omits `reason_bits`) to
demonstrate older-client compatibility.

## Compatibility

`required_oracle_version = 0` represents old clients that do not enforce a
contract-version floor. New clients against old contracts must either run with
that compatibility setting or reject with `unsupported_version`; they must not
guess by treating a failed version probe as safe.

No LedgerLens core ABI, storage key, event, error discriminant, or
cryptographic transcript changes are introduced by these fixtures. The added
types and errors are mock-consumer local. The `reason_bits` field is additive
and optional, so it does not break the ABI of existing `RiskScore` readers.

## Operations

Monitor per-client counts for `oracle_unavailable`, `unsupported_version`,
`reject_stale`, and `reject_low_confidence`. Recovery is configuration-only:
rotate `set_risk_oracle` to a healthy deployment, lower
`required_oracle_version` only for a documented compatibility rollback, or
increase `max_staleness_secs` only under an incident policy. For risk-policy
rollback, switch from `fail_open` back to the default `fail_closed` once the
oracle is healthy.

## Generated TypeScript Client

The first-party TypeScript client is generated from the same schema artifacts
that drive the Rust and Python SDKs (`tools/schema-gen`), so all three SDKs
share one source of truth for the observable contract. The generated package
must satisfy the fixtures above before it is published.

### Generation

- Typed bindings are generated for every public function and type exposed by
the schema artifacts, including event decoders and error mapping.
- The generated output is checked in and regenerated in CI; a drift check fails
the build if the committed bindings do not match the schema artifacts.
- CI runs the generated client against `tests/composability/sdk_conformance_fixtures.json`
  so the TypeScript SDK is verified against the same conformance fixtures as the
  Rust and Python SDKs.

### Read helpers

- Score reads expose staleness helpers so callers can distinguish a fresh score
  from a stale one without re-implementing the freshness math.
- Gate helpers wrap the LedgerLens gate call and return the explicit fixture
  outcomes (`allow`, `reject_high_risk`, `reject_low_confidence`,
  `reject_stale`, `oracle_unavailable`, `unsupported_version`).
- Pagination iterators are provided for list-style reads so integrators do not
  hand-roll cursor handling.

### Fail-closed decision helpers

The TypeScript client mirrors the Rust consumer crate's fail-closed decision
helpers. Transport failure is never reported as low risk: the default policy is
`fail_closed`, and `fail_open` is only honored when configuration explicitly
selects availability over risk freshness. Risk rejections remain distinct from
operational failures, matching the outcome table above.

### Publishing and provenance

The package is published to npm from CI using provenance attestation, so the
published package's provenance can be verified from the registry. The release
process is tied to contract interface versions, and each release records the
contract interface version it was generated from.

### Version compatibility

| Client | Contract interface | Behavior |
| --- | --- | --- |
| `required_oracle_version = 0` | Any | Compatibility mode; no version floor enforced. |
| `required_oracle_version > 0` | `>= required_oracle_version` | Normal operation. |
| `required_oracle_version > 0` | `< required_oracle_version` | Reject with `unsupported_version`. |

New clients against old contracts must either run with the compatibility
setting or reject with `unsupported_version`; they must not guess by treating a
failed version probe as safe.

### Quickstart

1. Install the generated package from npm.
2. Configure the LedgerLens contract ID, admin, and the desired failure policy
   (`fail_closed` by default).
3. Use the generated read helpers to fetch a score and check staleness, then
   call the gate helper to obtain an explicit decision outcome.
4. Run the conformance fixtures locally to confirm the client matches the
   observable contract before deploying.

## Generated Python Client

The first-party Python client is generated from the same schema artifacts that
drive the Rust and TypeScript SDKs (`tools/schema-gen`), so all three SDKs share
one source of truth for the observable contract. The generated package must
satisfy the fixtures above before it is published.

### Generation

- Dataclass-based models, error enums, and a client wrapper are generated for
  every public function and type exposed by the schema artifacts, including
  event decoders and error mapping.
- The generated output ships full type stubs with a `py.typed` marker so the
  package is usable under strict mypy, and it targets the supported Python
  versions.
- The generated output is checked in and regenerated in CI; a drift check fails
  the build if the committed bindings do not match the schema artifacts.
- CI runs the generated client against `tests/composability/sdk_conformance_fixtures.json`
  so the Python SDK is verified against the same conformance fixtures as the
  Rust and TypeScript SDKs.

### Canonical encoding

- Attestation payloads are encoded canonically so the off-chain service and the
  chain agree byte-for-byte; the encoding is validated against the shared
  cross-language fixtures, including negative vectors.
- Cross-language attestation vectors verify identically in Rust, TypeScript and
  Python.

### Read helpers

- Score reads expose staleness helpers so callers can distinguish a fresh score
  from a stale one without re-implementing the freshness math.
- Gate helpers wrap the LedgerLens gate call and return the explicit fixture
  outcomes (`allow`, `reject_high_risk`, `reject_low_confidence`,
  `reject_stale`, `oracle_unavailable`, `unsupported_version`).
- Pagination iterators are provided for list-style reads so integrators do not
  hand-roll cursor handling.

### Fail-closed decision helpers

The Python client mirrors the Rust consumer crate's fail-closed decision
helpers. Transport failure is never reported as low risk: the default policy is
`fail_closed`, and `fail_open` is only honored when configuration explicitly
selects availability over risk freshness. Risk rejections remain distinct from
operational failures, matching the outcome table above.

### Publishing and provenance

The package is published to PyPI from CI using trusted publishing, so the
published package's provenance can be verified from the registry. The release
workflow is validated in a dry run before publishing. The release process is
tied to contract interface versions, and each release records the contract
interface version it was generated from.

### Version compatibility

| Client | Contract interface | Behavior |
| --- | --- | --- |
| `required_oracle_version = 0` | Any | Compatibility mode; no version floor enforced. |
| `required_oracle_version > 0` | `>= required_oracle_version` | Normal operation. |
| `required_oracle_version > 0` | `< required_oracle_version` | Reject with `unsupported_version`. |

New clients against old contracts must either run with the compatibility
setting or reject with `unsupported_version`; they must not guess by treating a
failed version probe as safe.

### Quickstart

1. Install the generated package from PyPI.
2. Configure the LedgerLens contract ID, admin, and the desired failure policy
   (`fail_closed` by default).
3. Use the generated read helpers to fetch a score and check staleness, then
   call the gate helper to obtain an explicit decision outcome.
4. Sign and submit an attested score using the canonical encoding helpers, then
   run the conformance fixtures locally to confirm the client matches the
   observable contract before deploying.

## PR Design Notes

Trust assumptions: consumers trust the configured LedgerLens contract ID and
the configured admin, not arbitrary callback contracts.

Authorization boundary: only the mock fixture admin can rotate the oracle or
change thresholds, freshness, version, and failure policy.

State transitions: initialize stores admin and default fail-closed config;
configuration calls atomically replace the relevant instance-storage fields;
swap/borrow paths perform reads only.

Failure modes: high risk, low confidence, stale data, unavailable oracle,
unsupported version, malformed response, and unauthorized configuration all map
to explicit outcomes.

Rejected alternatives: collapsing every failure to a boolean was rejected
because transport failure could masquerade as low risk; adding LedgerLens core
ABI/storage changes was rejected because the existing gate and score APIs
already expose the needed signals. Extending the `RiskScore` struct with
`reason_bits` was rejected in favor of an optional submission field plus a
sibling read path, because a struct change would alter the ABI for existing
readers; the sibling approach keeps the footprint to one optional field and
preserves unknown bits for forward compatibility.

Invariant protected: no consumer action proceeds unless the configured policy
and current oracle evidence explicitly permit it.

Closes #679
Closes #1132
