# Unsafe Code Policy, Miri and Sanitizer Coverage

The off-chain tool crates parse untrusted input (replay snapshots, RPC
responses, exports), so undefined behaviour or memory errors there are treated
as security bugs. Workflow: `.github/workflows/sanitizers.yml`.

## Policy

- Every tool crate root declares `#![forbid(unsafe_code)]`. Adding `unsafe`
  to a tool crate requires removing the attribute in review, a `// SAFETY:`
  comment on each block, and a new row in the table below.
- Contract crates compile to WASM and contain no `unsafe` in production code.

## Unsafe usage inventory

| Location | Scope | Justification |
|---|---|---|
| `contracts/ledgerlens-score/src/test_gate_fee.rs` (2 × `transmute`) | `#[cfg(test)]` only | Lifetime-extends a generated test client; never compiled into WASM. |
| `contracts/ledgerlens-score/src/test_portfolio_var.rs` (1 × `transmute`) | `#[cfg(test)]` only | Same pattern as above. |
| `tools/*` | none | `#![forbid(unsafe_code)]` enforced at every crate root. |

## Coverage matrix

| Crate | Miri | ASan + LSan | Notes |
|---|---|---|---|
| `replay` | yes (`--lib`) | yes | Pure-Rust parsing and determinism logic. |
| `schema-gen` | yes (`--lib`) | yes | Pure-Rust code generation. |
| `recovery` | no | yes | Binary-only crate driving the Soroban test host; Miri is prohibitively slow and the host uses unsupported intrinsics. |
| `invocation-fuzzer` | no | yes | Runs thousands of contract invocations through the Soroban host; infeasible under Miri's interpreter. |

Rust has no UndefinedBehaviorSanitizer; UB is covered by Miri for the pure
crates and by `-Zub-checks=yes -Cdebug-assertions=yes` (precondition checks in
`core`/`std` unsafe APIs) in the ASan jobs for the rest.

## Triggers

- Nightly at 03:17 UTC and via `workflow_dispatch`.
- On pull requests only when labelled `security`.

## Self-test

`sanitizer-self-test` compiles a program with a seeded heap out-of-bounds read
and fails unless ASan reports `heap-buffer-overflow`, proving the toolchain and
flags actually detect memory errors.
