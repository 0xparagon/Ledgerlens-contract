# Test tiers

**Status:** normative for the repository's test layout. Introduced by issue #1244.

This document is the prose companion to [`test-tiers.toml`](../test-tiers.toml). The
manifest is machine-readable and authoritative: it says what test exists, why it exists, and
which run executes it. This document explains the model, the workflow, and the policy. Where
the two disagree, the manifest wins and this file is the bug.

The single command a contributor needs is:

```bash
scripts/test-tier.sh fast        # 3-minute feedback loop
scripts/test-tier.sh standard    # everything correct on every PR
scripts/test-tier.sh nightly     # the slow, broad remainder
```

## The problem this solves

Before tiers existed, the repository had exactly one way to run tests: `cargo test`, plus a
list of shell and Python suites in a CONTRIBUTING checklist that nothing enforced. Three
problems followed from that, all of them observable in the tree before this change:

1. **Feedback time was unbounded.** 962 test functions sat in one undifferentiated suite, so
   a one-line change and a cross-contract integration suite cost the same wall-clock time.
2. **A large amount of test code had never run.** 87 test units in the tree were not
   compiled by any cargo target: 81 modules in `contracts/ledgerlens-score/src/` (17 whose
   `mod` declaration is commented out, 64 never declared), 2 in
   `contracts/ledgerlens-aggregator/src/`, 3 in `tools/replay/src/`, and
   `tests/doc_sync_check.rs`, which belongs to no cargo target at all. Their tests are
   plausible-looking and dead. Nothing in CI noticed, because nothing compared the files on
   disk to the targets cargo builds.
3. **Slow tests had no scheduled home.** Resource-exhaustion, hostile-read and long-horizon
   soak suites either ran on every PR or not at all. There was no tier for "correct, but not
   on every PR", so the choice was between a slow PR and a skipped suite.

## Model

### Test units

A **test unit** is the granularity the tiers are defined over. A unit is one of:

| Kind of unit | Example | Cargo target it runs in |
|---|---|---|
| `#[cfg(test)]` module of a library | `contracts/ledgerlens-score/src/test_upgrade.rs` | `--lib` |
| Integration-test target | `tests/composability/tests/sdk_conformance.rs` | `--test sdk_conformance` |
| Doctest target | `contracts/ledgerlens-score::doctests` | `--doc` |
| Shell or Python suite | `tests/deploy/test_deploy.sh` | none — a recorded command |
| Stray test file under `tests/` | `tests/doc_sync_check.rs` | none (currently `disabled`) |

Support modules are deliberately *not* units: a file inside a subdirectory of a package's
`tests/` dir (e.g. `tests/common/mod.rs`) is compiled *into* another target, and a crate root
is not a test. Recording either as a unit would mean asking the runner to execute targets that
do not exist.

Discovery is mechanical and lives in `tools/test-tiering.py`:

- every `src/*.rs` in a workspace package that contains a test attribute is a candidate; it is
  **wired** if `src/lib.rs` declares its module, and the manifest records that as its tier;
- every `tests/<name>.rs` directly under a package's `tests/` dir is a wired integration
  target; a file in a subdirectory there is a support module and is skipped;
- a doctest unit exists for a package only if a doc comment in it contains a fenced example;
- everything under `tests/` counts as a test whatever it is called (`tests/deploy_script_rpc_failures.sh`
  is a failure-mode suite despite not saying "test"), while outside `tests/` only
  `*.test.sh` and `test_*.py` count, so checkers in `tools/` are not mistaken for tests;
- a `.rs` file under `tests/` that no package owns is a stray.

### Kinds

Every unit carries a `kind` — its purpose — and a `tier` — what runs it. The kinds and their
default tiers:

| Kind | Purpose | Default tier |
|---|---|---|
| `smoke` | The "if this breaks, stop" set: core entry points, the advertised interface, the error surface, executable invariants, runnable doc examples | `fast` |
| `unit` | One function or one storage key in isolation: validation branches, getters, setters, event shape | `fast` |
| `integration` | More than one contract, or one contract's lifecycle: composability, conformance, failover, governance/upgrade flows, replay and deploy tooling | `standard` |
| `property` | Invariant sweeps and property-based suites: schema↔discriminants, fail-closed truth tables, HLL/GDPR accumulators | `standard` |
| `stress` | Adversarial resource and shape: high-cardinality loops, hostile read patterns, memory exhaustion, proof corpora | `nightly` |
| `soak` | Long simulated horizons: TTL and rent management, retention sweeps | `nightly` |
| `mutation` | Suites that exist to kill cargo-mutants mutants | `nightly` |

The tiers **partition** the suite. `fast ∪ standard ∪ nightly` is exactly the set of units a
cargo target compiles; a unit belongs to exactly one tier.

### Deviations

A unit may sit in a different tier from its kind's default, but only with a `reason` on the
unit. A handful do, and the reason is what makes the deviation reviewable rather than
accidental:

| Unit | Default | Actual | Why |
|---|---|---|---|
| `contracts/ledgerlens-score/src/test_interface.rs` | `fast` | `standard` | A 1000-round loop over a wallet set; the cheapest honest home is `standard`. |
| `contracts/ledgerlens-score::doctests` | `fast` | `standard` | Compiles the whole crate's examples; a 3-minute budget is not the place to find that out. |
| `tests/test_traceability.py` | `fast` | `standard` | Spawns the replay traceability report once per case; tool-level coverage, not the contract's critical path. |
| `tools/schema-gen::doctests` | `fast` | `standard` | Compiles the schema generator's examples; same reasoning as the contract doctests. |

The bootstrap heuristics propose a `kind` and a `tier`; they do not own them. Every hand
classification in the manifest carries a `note` explaining what the heuristic got wrong —
`test_privacy_regression_history.rs` is a `unit` suite because "history" there means a
regression *record*, not a simulated time horizon, for example.

## Budgets

| Tier | Execution budget | Build budget (not enforced) | Runs on |
|---|---|---|---|
| `fast` | 180s (3 min) | 900s | every PR, `push` to `main`, pre-push |
| `standard` | 1800s (30 min) | 900s | every PR, `main`, local |
| `nightly` | 3600s (60 min) | 1800s | nightly cron, manual dispatch |

**The budget is on execution, not on build.** `scripts/test-tier.sh` pre-builds the tier's
test targets once and reports that cost separately as `build_budget_seconds`. A cold build of
this workspace under `lto = true` is dominated by compilation, not by anything under test, so
folding it into the budget would measure the toolchain rather than the tests.

**The budgets are provisional.** They are the ceilings this change intends to hold, not
ceilings derived from a measured distribution, because the first measured distribution only
exists once the tiers are wired. Treat the first `standard` and `nightly` runs as calibration:
if a tier lands far under its budget, re-baseline it (a budget change requires a `reviewed`
annotation, so this is a deliberate act, not a drift). If a tier lands over, the interesting
question is which unit pushed it over — the report ranks units by measured cost precisely so
that question has an answer.

**Reference runner.** Budgets are wall-clock ceilings for a GitHub-hosted `ubuntu-latest`
(2 vCPU, 7 GB RAM) on the toolchain in `rust-toolchain.toml`. A laptop is not that machine.
`scripts/test-tier.sh --no-budget` reports the budget without failing on it, for local runs
on slower hardware.

## Commands

```bash
scripts/test-tier.sh <fast|standard|nightly> [options]
```

| Option | Effect |
|---|---|
| `--base <rev>` | Diff the manifest against `<rev>` and fail on an unreviewed tier or budget change. CI passes the PR base SHA. |
| `--no-budget` | Report the budget, do not fail on it. For local machines slower than the reference runner. |
| `--no-build` | Skip the pre-build step. |
| `--fail-fast` | Stop at the first failing unit instead of running the whole tier. |
| `--list` | Print the units in the tier and exit. |
| `--dry-run` | Print the commands the tier would run and exit. |
| `-- <args>` | Append arguments to every command in the tier (`-- --nocapture`). |

What the runner does, in order:

1. asks `tools/test-tiering.py plan --tier <tier>` for the tier's unit list, which comes from
   the manifest — the runner hard-codes no test list;
2. pre-builds the tier's targets once, timed separately;
3. runs **each unit as its own command**, so the report can attribute cost to a unit rather
   than to a tier as a whole. Per-unit commands also mean a hung suite is attributable;
4. writes `target/test-tiers/<tier>-results.tsv`, one row per unit with its status and
   duration, and per-unit logs under `target/test-tiers/logs/<tier>/`;
5. renders `target/test-tiers/<tier>-report.md` and exits non-zero if a unit failed, a unit
   did not run, the tier exceeded its budget, or a tier moved without review.

The report is the artifact to paste into a PR that touched a tier.

### Provider

The planner uses `cargo nextest run` when `cargo-nextest` is installed and the built-in
`cargo test` harness otherwise; per-unit timings are the reason, and nextest is much better at
them. Two exceptions are deliberate:

- **Doctests always use `cargo test --doc`.** cargo-nextest does not run doctests at all, so a
  nextest-only plan would silently skip a whole unit.
- **Integration targets are filtered by binary, not by test name.** A `tests/<name>.rs` file
  is its own target and its test names are not namespaced by the file stem, so
  `--test <name>` (or `-E 'binary(<name>)'` under nextest) is the correct filter. `--lib
  <stem>::` would match nothing.

## CI

| Job | Workflow | Command |
|---|---|---|
| `test-tiers-manifest` | `.github/workflows/ci.yml` | `python3 tools/test-tiering.py check --base <base sha>` |
| `test-tiers-fast` | `.github/workflows/ci.yml` | `scripts/test-tier.sh fast --base <base sha>` |
| `test-tiers-standard` | `.github/workflows/ci.yml` | `scripts/test-tier.sh standard --base <base sha>` |
| `nightly` | `.github/workflows/nightly-tests.yml` | `scripts/test-tier.sh nightly` |

The manifest gate is cheap (no compilation) and runs first, so a misclassified test fails in
seconds rather than after a full suite.

### What the tiers replaced

The tiers are the only place a test runs now, so the standalone jobs that used to invoke
those same suites were removed rather than left to duplicate the work:

| Removed job | Its step is now run by |
|---|---|
| `test` (`cargo test --workspace`) | `test-tiers-fast` + `test-tiers-standard` |
| `deploy-script` | `tests/deploy/test_deploy.sh` in the standard tier |
| `deploy-script-tests` | `tests/deploy_script_rpc_failures.sh` in the standard tier |
| `refinement-mapping` | `tests/check_refinement_mapping.sh` in the fast tier |

Two jobs that are *not* tests stayed where they are: `error-discriminants` keeps its
base-vs-HEAD discriminant comparison (it needs the PR base SHA, so it is not a tier unit —
its checker's own test suite is a fast-tier unit), and `production-acceptance` produces a
release-readiness report rather than a pass/fail suite.

## Classification workflow

### Adding a test

1. Write it. If it is a new module in a package's `src/`, also add its `mod` declaration —
   the file is inert otherwise, which is exactly the failure mode this manifest now detects.
2. `python3 tools/test-tiering.py bootstrap --write` to add it to the inventory.
3. Read the proposed `kind` and `tier`. Fix them if the heuristic got it wrong, and add a
   `note` saying why. If the tier differs from the kind's default, add a `reason`.
4. `python3 tools/test-tiering.py check` must pass before you push.

A new test that is not in the manifest fails the gate. So does a newly *wired* module that is
not in the manifest: un-commenting a `mod` line in `src/lib.rs` without classifying the
module fails `test-tiers-manifest`.

### Moving a test between tiers

A tier move is a policy change, not a refactor, so it needs an annotation on the same entry:

```toml
[[unit]]
id = "contracts/ledgerlens-score/src/test_verkle.rs"
kind = "mutation"
tier = "nightly"
reviewed = { by = "maintainer", on = "#1287", note = "27 proof-system tests; they are the kill-suite for shard-e, not PR feedback" }
```

`check --base <rev>` and `report --base <rev>` both fail when a unit's `kind` or `tier`, or a
tier's budget or command, changed without one. `--base` accepts a git revision or a path to a
manifest, so you can diff against a local copy of the pre-change file.

The same rule applies in reverse: a `disabled` unit that becomes live is a tier move, and so
is un-wiring a live suite back to `disabled` (which additionally requires a `reason` naming
the evidence — the check reads the tree and tells you which of the two states it is in).

## Disabled inventory

87 units are recorded as `disabled`: present in the tree, not compiled by any cargo target.
They are listed in `test-tiers.toml` with the evidence for their state — `no mod declaration
in src/lib.rs`, `mod declaration is commented out in src/lib.rs`, or `under tests/ but inside
no cargo package target` — rather than being left as an unremarked fact.

This is a state the repository was in, not one this change created, and the manifest's job is
to pin it so it cannot change silently. Triage is follow-up work:

1. **8 of the 14 `cargo-mutants` kill-suites are unwired.** `.cargo/mutants.toml` shards
   `shard-a` … `shard-e` run mutants with `--test <name>` for suites that no cargo target
   compiles (`test_rate_limit`, `test_score_floor`, `test_zk_range_proof`,
   `test_upgrade_multisig`, `test_param_timelock`, `test_adaptive_rate_limit`,
   `test_rate_limit_override_log`, `test_is_score_floor_enabled`). Those mutants cannot be
   killed by those suites. `check` reports this as a **warning**, not a violation: it is a
   pre-existing defect in the mutation configuration, and a PR that did not cause it should
   not be blocked by it. Fixing it means deciding whether those suites still compile at all
   before wiring them back up.
2. **Nine suites cited by [`docs/invariants.md`](invariants.md) as enforcing non-negotiable
   invariants are themselves unwired**, so those citations overstate current enforcement. The
   document now carries a caveat saying so; the fix is triage, not a doc edit.
3. **The remaining 70 unwired modules** need triaging into live tiers or deletion. Some are
   genuinely valuable coverage that was lost to a bad merge; some are superseded by newer
   suites. This is deliberate work with a reviewer, not a mechanical edit — the bootstrap
   cannot classify a suite that does not compile.

## Benchmarks

The 16 `[[benchmark]]` entries in the manifest are classified and checked for existence, but
deliberately sit outside the tiers. A criterion benchmark in this workspace needs a release
build with `lto = true`, which costs more wall-clock time than the entire `nightly` execution
budget and measures the optimiser rather than the code. Benchmark regression tracking stays
where it is; mixing it into a tier budget would make the budget meaningless.

## What this change does not do

- It does not re-calibrate the budgets from measured runs (§ Budgets).
- It does not wire, delete, or fix the 87 disabled units (§ Disabled inventory).
- It does not change any public ABI, storage layout, error discriminant, or event. No contract
  code is modified by this change.
- It does not replace `cargo test` as a debugging tool. The tiers exist to bound feedback
  time and to make the inventory honest, not to remove the ability to run one suite directly.
