# Contributing to LedgerLens Contract

Thanks for your interest in improving the LedgerLens on-chain risk score registry.

## Quick Start

1. Clone the repo and install the toolchain:
   ```bash
   git clone https://github.com/Ledger-Lenz/Ledgerlens-contract.git
   cd Ledgerlens-contract
   rustup target add wasm32-unknown-unknown
   ```

2. Build the contract:
   ```bash
   cargo build --all
   # WASM release build:
   cargo build --target wasm32-unknown-unknown --release
   ```

3. Run the fast test tier:
   ```bash
   scripts/test-tier.sh fast
   ```

4. Make a small change (e.g., fix a typo or add a doc example):
   - Edit files inside `contracts/ledgerlens-score/`
   - Run `scripts/test-tier.sh fast` to verify your change
   - The pre-PR checks (fmt, clippy, tiers, wasm build) are documented in the **[## Before Opening a Pull Request](#before-opening-a-pull-request)** section below.

## Opening an Issue

Pick the template that matches what you're actually doing — each one asks for the specific
evidence that class of task needs, not a generic description box:

| Task class | Template | What it's for |
|---|---|---|
| Implementation | [`🔧 Implementation task`](.github/ISSUE_TEMPLATE/implementation-task.yml) | Adding or changing contract behavior. |
| Testing | [`🧪 Testing task`](.github/ISSUE_TEMPLATE/testing-task.yml) | Closing a coverage gap without changing behavior. |
| Documentation | [`📚 Documentation task`](.github/ISSUE_TEMPLATE/documentation-task.yml) | README/CONTRIBUTING/docs additions or corrections. |
| Security review | [`🔒 Security review task`](.github/ISSUE_TEMPLATE/security-review-task.yml) | Auditing a specific property across a defined set of code paths. |
| Benchmark | [`📊 Benchmark task`](.github/ISSUE_TEMPLATE/benchmark-task.yml) | Measuring and recording resource cost, especially at a worst-case bound. |

Every template requires an explicit **Compatibility impact** statement (even "None") and links
back to [`docs/invariants.md`](docs/invariants.md) — read that first regardless of which template
you use.

Touching governance, cryptography, storage, upgrades, or composability? Work through
[`docs/review-checklists.md`](docs/review-checklists.md) — short, actionable per-category gates
reviewers will hold your PR to.

## Getting Started

1. Install the Rust toolchain (stable) and the `wasm32-unknown-unknown` target:
   ```bash
   rustup target add wasm32-unknown-unknown
   ```
2. Fork the repo and create a feature branch off `main`.
3. Make your changes inside `contracts/ledgerlens-score/`.

## Before Opening a Pull Request

Run the same checks CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
python3 tools/test-tiering.py check        # is every test classified?
scripts/test-tier.sh fast                  # 3-minute execution budget
scripts/test-tier.sh standard              # 30-minute execution budget
cargo build --target wasm32-unknown-unknown --release
cargo vet
```

`cargo test` still works for debugging a single suite; the tiers exist to bound feedback time,
not to remove that.

## Test tiers

Every test in this repository is listed in [`test-tiers.toml`](test-tiers.toml) with a *kind*
(what it is for) and a *tier* (what runs it). Three tiers exist, each with a wall-clock
execution budget, and they partition the suite:

| Tier | Budget | Runs on |
|---|---|---|
| `fast` | 3 minutes | every PR, `main`, pre-push |
| `standard` | 30 minutes | every PR, `main`, local |
| `nightly` | 60 minutes | nightly schedule, manual dispatch |

```bash
scripts/test-tier.sh fast        # what CI runs on every PR
scripts/test-tier.sh standard    # the rest of the per-PR coverage
scripts/test-tier.sh --help      # --list, --dry-run, --no-budget, --base, --fail-fast
```

Use `--no-budget` locally: the budgets are ceilings for a GitHub-hosted 2-vCPU runner, and a
laptop is not that machine. Without it you still get the per-unit cost report.

**Adding or moving a test** means editing the manifest, and `python3 tools/test-tiering.py
check` is the gate CI runs (it is cheap — it compiles nothing):

```bash
# 1. add a `mod test_foo;` line to src/lib.rs, and write the test
# 2. let the tool add it to the inventory
python3 tools/test-tiering.py bootstrap --write
# 3. read what it proposed: `kind` is what the test is for, `tier` is what runs it
# 4. if the tier differs from the kind's default, add a `reason` saying why
# 5. the gate must pass
python3 tools/test-tiering.py check
```

A test that no tier runs fails the gate, and so does a newly wired module that was not
classified. Moving a unit between tiers, or changing a budget, additionally requires a
`reviewed = { by, on, note }` annotation on that entry — `check --base <rev>` fails without
one, so a slow suite cannot be quietly demoted out of PR coverage. See
[`docs/test-tiers.md`](docs/test-tiers.md) for the model, the full taxonomy, and how to read a
tier report.

## Guidelines

- **Read [`docs/invariants.md`](docs/invariants.md) before touching `lib.rs`.** It lists the
  behaviors that are non-negotiable — fail-closed gates, no-panic reads, bounded storage, and
  append-only event/error stability — with pointers to exactly what enforces each one. If your
  change would weaken any of them, that needs a design discussion before a PR, not after.
- Keep `contracts/ledgerlens-score/src/types.rs` changes minimal and deliberate — `RiskScore` and `DataKey` are shared, cross-repo data contracts (see [README.md § Organization Architecture](README.md#organization-architecture)). Any field/shape change is breaking for the `api`, `core`, and `dashboard` repos and must be coordinated. When adding new storage keys, follow the storage-key partitioning guidelines documented in [**ADR 0001: Storage-Key Enum Partitioning**](docs/adr/0001-storage-key-enum-split.md):
  1. Add domain-specific keys to their dedicated enum if one exists (e.g. `GateDataKey`).
  2. For general or extension storage keys, add to `DataKeyD` while its variant count remains below 50 (currently 39 variants).
  3. When `DataKeyD` reaches 50 variants, route new keys to `DataKeyE` (currently 1 variant) or create a dedicated subsystem enum.
  4. Never remove, rename, or move existing variants across enums (violates on-chain storage key derivation and orphans deployed state).
- Add or update tests in `src/test.rs` for any behavioral change, and classify any new test
  module in [`test-tiers.toml`](test-tiers.toml) (see **[## Test tiers](#test-tiers)**). A new
  `#[cfg(test)]` module is dead code until `src/lib.rs` declares it, and unclassified until the
  manifest says which tier runs it.
- For replay or forensic workflow changes, add or update tests in `tools/replay/src/main.rs` and keep the evidence bundle deterministic and stable across reruns.
- Keep error codes in `errors.rs` stable; append new variants rather than reordering or removing existing ones, since their numeric values are part of the deployed contract's ABI. This is enforced in CI by the `error-discriminants` job (`tools/check_error_discriminants.sh`), which fails the build if a PR renames, removes, or renumbers any discriminant that already existed on the base branch. New discriminants and new `pub const` aliases are always fine — prefer an alias over renumbering when you need a new name for an existing error.
- Update `README.md` if you change contract function signatures, events, or the deployment flow in `deploy.sh`.
- **Docs site.** `docs/` is published as a versioned mdBook site (`.github/workflows/docs.yml`): `latest/` from `main`, one snapshot per `v*` tag, and a preview per same-repo PR. Build it locally with `python3 tools/docs-site/build.py && mdbook build target/docs-site`. New pages are placed in the sidebar by audience via the keyword table in `tools/docs-site/build.py`; every page is stamped with the contract version it applies to (override with a first-line `<!-- applies-to: ... -->`). Broken internal links fail CI (`python3 tools/docs-site/check_links.py`); external links are checked by lychee with `.lycheeignore` as the allowlist.
- **Documentation snippets are executable.** `python3 tools/check_doc_snippets.py` (CI job `doc-snippets`) checks the README and integration guides: every `` ### `fn(args) -> Ret` `` heading must match the contract signature (`Result<T, Error>` is written as `T`), every `pub struct` listing must match the source fields, and shell snippets must parse with `bash -n`. Mark fenced blocks in the info string:
  - `` ```bash `` / `` ```rust `` (default, *compile*): static checks only. Use `<UPPER_CASE>` for placeholders.
  - `` ```bash run ``: additionally executed with `bash -euo pipefail` in an empty temp dir, scrubbed env, 30s timeout. Must not need the network.
  - `` ```bash ignore ``: skipped. The line directly above the fence must be `<!-- snippet-ignore: <justification> -->`, or CI fails.
- Use terms as defined in [`docs/glossary.md`](docs/glossary.md) consistently — e.g. don't call a `ledgerlens-aggregator` peer a "node" or a "partition" when the established term is **shard**; don't use "finality" to mean ledger-close finality when this repo's docs mean the finality *buffer* (a score-submission hold window). If you introduce a genuinely new concept, add it to the glossary in the same PR rather than letting a new term go undefined.
- **Interface-breaking changes** (see [`docs/interface-versioning-policy.md`](docs/interface-versioning-policy.md) for what counts as breaking) require a minimum 30-day notice period between the `Unreleased` changelog entry and mainnet deployment. The announcement must include a migration guide in `CHANGELOG.md`.

## Submitting a Pull Request

- Describe what changed and why.
- Note any cross-repo coordination needed (e.g. "requires `api` to update its `RiskScore` schema").
- Ensure all CI checks pass.
- **If your change touches governance, cryptography, storage, upgrades, or composability**, work
  through the matching checklist in [`docs/review-checklists.md`](docs/review-checklists.md)
  before requesting review — reviewers will be checking against it, so a PR description that
  addresses each item up front gets reviewed faster than one that doesn't.
