# CI Gate, Path Filters and Concurrency

## Required check

Branch protection on `main` requires exactly one status check: **`CI gate`**
(job `ci-gate` in `.github/workflows/ci.yml`). It runs with `if: always()`,
depends on every other job, and:

- passes when every dependency succeeded or was **skipped** (path-filtered or
  event-conditional jobs);
- fails when any dependency **failed** or was **cancelled**.

Individual job names are therefore not referenced by branch protection, and
renaming or adding a job does not block merges.

## Path filters

The `changes` job (`dorny/paths-filter`) exposes outputs used by `if:` on
heavy jobs:

| Output | Paths | Gated jobs |
|---|---|---|
| `rust` | `contracts/**`, `tools/**`, `tests/**`, `scripts/**`, `deploy/**`, `Cargo.*`, `rust-toolchain.toml`, `deny.toml`, `ci.yml` | fmt, clippy, lints, tests, build, audit, license, supply-chain, reproducible builds, production acceptance |
| `spec` | `spec/**`, `ci.yml` | TLA+ model check |

Cheap checks (deploy-script, refinement mapping, error discriminants) always
run. On pushes to `main` every output is forced to `true`, so the full
pipeline always runs on the default branch.

Dry-run examples:

| PR changes | `rust` | `spec` | Result |
|---|---|---|---|
| `docs/foo.md` only | false | false | heavy jobs skipped, gate passes in about a minute |
| `contracts/ledgerlens-score/src/lib.rs` | true | false | full Rust pipeline |
| `spec/LedgerLens.tla` | false | true | TLC only |
| `.github/workflows/ci.yml` | true | true | everything |

## Concurrency

Runs are grouped by PR number (pull requests) or commit SHA (pushes). A new
push to a PR cancels its superseded run; pushes to `main` and tags always
have a unique group and are never cancelled.

## Adding a new job

1. Add the job to `ci.yml`.
2. If it is expensive and only relevant to some paths, add
   `needs: changes` and `if: needs.changes.outputs.<filter> == 'true'`
   (add a new filter to `changes` if needed).
3. **Add the job id to `ci-gate.needs`.** A job missing from that list is not
   enforced.
4. Do not add the job to branch protection; `CI gate` covers it.
