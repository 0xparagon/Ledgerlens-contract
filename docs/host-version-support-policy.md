# Soroban host / Rust toolchain support policy

This repository supports one build toolchain for deployable Soroban WASM:

- Tested and supported: Rust `1.81.0`
- Explicitly unsupported: Rust `1.82.0` and newer

Why the boundary exists:

- `rust-toolchain.toml` pins the release toolchain to `1.81.0`.
- `1.82.0` flips `wasm32-unknown-unknown` defaults that emit WebAssembly
  features the current Soroban host does not accept.
- `.cargo/config.toml` keeps the local build and CI lint settings aligned so
  the same source tree produces the same deployable artifact in both places.

Policy:

| Status | Toolchain | Action |
|---|---|---|
| Tested | `1.81.0` | Supported for local builds and CI. |
| Supported | `1.81.0` | Expected version for reproducible release WASM. |
| Deprecated | none | There is no intermediate deprecation window for host/toolchain drift. |
| Unsupported | `1.82.0+` | Do not deploy WASM built with these toolchains. |

Compatibility checks and CI coverage:

- [`fmt`](../.github/workflows/ci.yml) — verifies formatting with the pinned toolchain.
- [`clippy`](../.github/workflows/ci.yml) — enforces lint cleanliness on the same toolchain.
- [`test`](../.github/workflows/ci.yml) — exercises the host-side contract tests.
- [`build`](../.github/workflows/ci.yml) — builds deployable WASM for `wasm32-unknown-unknown`.
- [`repro-build-1`](../.github/workflows/ci.yml) and
  [`repro-build-2`](../.github/workflows/ci.yml) — independent rebuilds used to
  verify byte-for-byte reproducibility.
- [`repro-verify`](../.github/workflows/ci.yml) — compares the two build artifacts.

The canonical reproducibility procedure is documented in
[`docs/reproducible-builds.md`](reproducible-builds.md).

Operational rule:

- If a local environment is on anything other than `1.81.0`, treat the build
  as non-deployable until the toolchain is aligned.
- If CI ever moves this boundary, update this policy, `rust-toolchain.toml`,
  and the workflow toolchain pins in the same PR.

SDK upgrades:

- The supported SDK is `soroban-sdk 21.0.0`. The staged migration to 22.x/23.x, including its toolchain
  change (`wasm32v1-none`, Rust ≥ 1.84), is tracked in [`soroban-sdk-migration.md`](soroban-sdk-migration.md).
- Candidate SDKs are built and compared on every manifest change by
  [`sdk-compat.yml`](../.github/workflows/sdk-compat.yml). See [`dependency-update-policy.md`](dependency-update-policy.md).
- WASM built with SDK 21 stays reproducible with the toolchain above after the migration. The last SDK-21
  artifact is kept in `tests/fixtures/historical/`.
