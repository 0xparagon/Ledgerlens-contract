# WASM Size Budget

This document defines the WASM size budget for the `ledgerlens-score` contract and
the size-tiered build profiles introduced for issue #1193.

## Build profiles

The contract exposes a Cargo feature graph so operators can compile out optional
subsystems they do not use. Each profile is a named combination of features and
has its own size budget and ABI snapshot.

| Profile    | Features enabled                                                        | Intended use                          |
| ---------- | ----------------------------------------------------------------------- | ------------------------------------- |
| `core`     | `core`                                                                  | Minimal scoring only                  |
| `standard` | `core`, `governance`, `privacy`                                         | Default operator deployment           |
| `full`     | `core`, `governance`, `privacy`, `proofs`, `oracle`, `dispute`          | All optional subsystems               |

### Feature dependency rules

- `core` is always enabled and has no dependencies on optional features.
- `governance`, `privacy`, `proofs`, `oracle` and `dispute` each depend on `core`.
- `dispute` additionally depends on `proofs` (dispute resolution consumes proof
  verification entry points).
- No feature silently changes the semantics of another: enabling a feature only
  adds its own entry points and never alters the behaviour of `core`.

When a feature is disabled its entry points are **not compiled into the ABI**
(rather than being present and failing at runtime), and `supports_interface`
reports only the interfaces available in the compiled profile.

## Size budget per profile

The budget is enforced in CI by the size-check job in the build matrix. The
budget is measured on the optimised release WASM artifact.

| Profile    | Size budget (KiB) |
| ---------- | ----------------- |
| `core`     | 180               |
| `standard` | 320               |
| `full`     | 512               |

A build that exceeds its profile budget fails the CI matrix job.

## Entry points per profile

The table below is generated from the ABI snapshots produced by the schema
generation step. Each profile has its own snapshot so the ABI diff gate compares
like-for-like and does not report spurious diffs between profiles.

| Entry point              | `core` | `standard` | `full` |
| ------------------------ | ------ | ---------- | ------ |
| `init`                   | yes    | yes        | yes    |
| `score`                  | yes    | yes        | yes    |
| `supports_interface`     | yes    | yes        | yes    |
| `set_governance_params`  | no     | yes        | yes    |
| `get_governance_params`  | no     | yes        | yes    |
| `set_privacy_policy`     | no     | yes        | yes    |
| `get_privacy_policy`     | no     | yes        | yes    |
| `submit_proof`           | no     | no         | yes    |
| `verify_proof`           | no     | no         | yes    |
| `set_oracle`             | no     | no         | yes    |
| `get_oracle`             | no     | no         | yes    |
| `open_dispute`           | no     | no         | yes    |
| `resolve_dispute`        | no     | no         | yes    |

## CI matrix

The CI workflow builds and tests each profile independently:

- `cargo build --no-default-features --features core`
- `cargo build --no-default-features --features standard`
- `cargo build --no-default-features --features full`

Each matrix entry runs the applicable test subset and the size-budget check for
that profile. The ABI diff gate and schema generation run per profile so each
profile keeps its own snapshot.

## Default release profile

The default release profile is unchanged unless an ADR explicitly decides
otherwise. The `standard` profile remains the default feature set for release
builds.
