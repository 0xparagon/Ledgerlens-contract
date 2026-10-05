# CI Caching and Wall-Clock Budget

How CI caches build outputs safely, how durations are tracked, and what the
time budget is (issue #1224).

## Caching strategy

| Layer | Tool | Key | Who may write |
|---|---|---|---|
| Dependencies and `target/` | `Swatinem/rust-cache@v2` | OS, rustc version (toolchain), job, `Cargo.lock` hash and every `Cargo.toml` | pushes to `main` only (`save-if`) |
| Compiler outputs | `sccache` (GitHub Actions cache backend) | rustc version, compiler flags, source hashes (computed by sccache per compilation unit) | GitHub cache scoping, see below |

`rust-cache` includes the resolved rustc version and the lockfile hash in its
key, so changing `rust-toolchain.toml`, the CI toolchain pin or `Cargo.lock`
produces a new cache instead of reusing a stale one. `CARGO_INCREMENTAL=0` is
set workflow-wide because incremental artifacts cannot be shared by sccache
and only inflate caches.

sccache is enabled for the compile-heavy jobs (clippy, unit tests, doctests
and benches, contract lints, WASM build, production acceptance). Coverage
builds use only `rust-cache` because instrumented builds are not reused
elsewhere.

### Fork PRs cannot write to trusted caches

* Every `rust-cache` step sets `save-if: github.ref == 'refs/heads/main'`.
  Pull requests, including those from forks, only **restore** caches.
* GitHub scopes caches by ref. Entries created while running a pull request
  live under `refs/pull/<n>/merge` and are visible only to re-runs of that
  PR; `main` and other PRs never read them. sccache writes on PRs therefore
  cannot poison what `main` builds with.
* The CI workflow's token is `contents: read` by default. Jobs that need more
  (`actions: read` to fetch artifacts, `pull-requests: write` for the coverage
  comment) request it per job, and fork PRs always receive a read-only token.

### Reproducible builds always build cold

`repro-build-1` and `repro-build-2` deliberately use no `rust-cache` and no
sccache. A guard step fails the job if `RUSTC_WRAPPER`,
`SCCACHE_GHA_ENABLED` or a pre-existing `target/` directory is present, so a
future edit cannot silently let a cache mask non-determinism. See
[reproducible-builds.md](reproducible-builds.md).

## Duration tracking

`.github/workflows/ci-metrics.yml` runs after every `Contract CI` run on
`main` and weekly. It runs `tools/ci/job_durations.py`, which:

1. fetches completed runs and their jobs from the GitHub API;
2. appends one record per run (run id, commit, wall clock, per-job seconds)
   to `ci-metrics/durations.jsonl`, a JSON-lines dataset persisted in the
   Actions cache and uploaded as the `ci-durations` artifact (90 days);
3. writes a trend table to the job summary comparing the median of the last
   10 runs with the previous 10.

Run it locally against the downloaded dataset:

```bash
python3 tools/ci/job_durations.py trend --dataset durations.jsonl --budget .github/ci-budget.json
```

## Wall-clock budget

Budgets live in [`.github/ci-budget.json`](../.github/ci-budget.json):

* **Required checks: 25 minutes** wall clock (first job start to last job
  end of a `Contract CI` run).
* Per-job budgets for the long jobs (for example 15 min per unit-test
  partition, 20 min for the production acceptance report).

When the latest run exceeds a budget the metrics workflow emits a
`CI wall-clock budget` warning annotation and lists it in the summary. Raise a
budget only with a justification in the PR that changes it.

## Before and after

Baseline on `main` before this change (three most recent `Contract CI` runs,
2026-09-07):

| Metric | Run 1 | Run 2 | Run 3 | Median |
|---|---|---|---|---|
| Required checks wall clock | 16.2 min | 16.4 min | 15.0 min | **16.2 min** |
| Unit tests job | 12.2 min | 10.9 min | 13.2 min | **12.2 min** |

The after numbers come from this change's CI runs and are recorded in its PR
description; the `CI metrics` workflow tracks them from then on.
