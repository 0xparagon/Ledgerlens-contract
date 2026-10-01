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

3. Run the full test suite:
   ```bash
   cargo test
   ```

4. Make a small change (e.g., fix a typo or add a doc example):
   - Edit files inside `contracts/ledgerlens-score/`
   - Run `cargo test` to verify your change
   - The four pre-PR checks (fmt, clippy, test, wasm build) are documented in the **[## Before Opening a Pull Request](#before-opening-a-pull-request)** section below.

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
cargo test
cargo build --target wasm32-unknown-unknown --release
cargo vet
```

## Dependency Audits (cargo-vet)

We use [`cargo-vet`](https://mozilla.github.io/cargo-vet/) to record which dependencies a human
has actually read, and to import trusted third-party audit sets so we only review what nobody
else has. The configuration lives in [`supply-chain/`](supply-chain/) and the `cargo-vet` CI job
fails the build when a new, unaudited dependency is added to a contract crate.

### Criteria

We use two criteria, and the distinction matters:

- **`safe-to-deploy`** — required for every crate linked into the WASM contract. These crates run
  on-chain, so a bug is a deployed bug. This is the stricter bar.
- **`safe-to-run`** — sufficient for dev-only and tooling crates (test helpers, `tools/replay`,
  build scripts). These never ship in the WASM, so the bar is lower.

### Trusted import sources

Rather than auditing everything ourselves, we import audit sets from projects we trust. The
sources are declared in `supply-chain/config.toml` under `[imports.*]`:

- `mozilla` — Mozilla's audit set, broad coverage of the Rust ecosystem.
- `bytecode-alliance` — audits from the Bytecode Alliance, which maintains the WASM toolchain
  crates we depend on.
- `google` — Google's audit set, covering common crates in the ecosystem.

Imports are only trusted for the criteria they were written against; a `safe-to-run` import does
not satisfy `safe-to-deploy` for a WASM-linked crate.

### Adding or renewing an audit

When `cargo vet` reports an unaudited crate, pick the option that matches the situation:

1. **Import it.** If a trusted source already audits the crate, add the source to
   `[imports.*]` in `supply-chain/config.toml` and re-run `cargo vet`. Prefer this — it costs us
   nothing and keeps our own audit list small.
2. **Audit it.** Read the crate and record the audit:
   ```bash
   cargo vet certify <crate> <version>
   ```
   This writes an entry to `supply-chain/audits.toml` with your name and the criteria you
   certified. Only certify criteria you have actually verified.
3. **Exempt it.** If the crate is low-risk and not worth a full audit, record a time-boxed
   exemption:
   ```bash
   cargo vet add-exemption <crate> <version>
   ```
   Then edit the generated entry in `supply-chain/config.toml` to add a `notes` field with the
   rationale and an owner, and set `suggested-date` to a review-by date no more than 12 months
   out. Exemptions are a debt, not a solution — every entry needs a rationale and an owner, and
   the list is reviewed when it comes due.

### Example: adding a new dependency

Say you add `hex = "0.4"` to `contracts/ledgerlens-score/Cargo.toml` and it is linked into the
WASM. `cargo vet` will fail with an unaudited crate. Work through the options above:

```bash
# 1. See what is missing and whether an import already covers it.
cargo vet

# 2a. If a trusted source covers it, add the import to supply-chain/config.toml
#     under [imports.<source>] and re-run:
cargo vet

# 2b. Otherwise audit it yourself (this records safe-to-deploy for hex 0.4.x):
cargo vet certify hex 0.4.3

# 3. Or, if it is genuinely low-risk, exempt it with a rationale and review-by date:
cargo vet add-exemption hex 0.4.3
#    then edit supply-chain/config.toml to add notes + suggested-date.
```

Commit the updated `supply-chain/` files with your change. The `cargo-vet` CI job re-runs on the
PR and will pass once every crate in the tree is covered by an import, an audit, or a documented
exemption.

### Exemption report

Every exemption in `supply-chain/config.toml` carries an owner and a review-by date in its
`notes`/`suggested-date` fields. To list the current exemptions and their owners:

```bash
cargo vet --locked 2>/dev/null; grep -A4 '\[exemptions\]' supply-chain/config.toml
```

Keep this list as short as possible: prefer an import or a real audit over an exemption, and
renew or remove each exemption before its review-by date.

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
- Add or update tests in `src/test.rs` for any behavioral change.
- For replay or forensic workflow changes, add or update tests in `tools/replay/src/main.rs` and keep the evidence bundle deterministic and stable across reruns.
- Keep error codes in `errors.rs` stable; append new variants rather than reordering or removing existing ones, since their numeric values are part of the deployed contract's ABI. This is enforced in CI by the `error-discriminants` job (`tools/check_error_discriminants.sh`), which fails the build if a PR renames, removes, or renumbers any discriminant that already existed on the base branch. New discriminants and new `pub const` aliases are always fine — prefer an alias over renumbering when you need a new name for an existing error.
- Update `README.md` if you change contract function signatures, events, or the deployment flow in `deploy.sh`.
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
