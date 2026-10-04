# Running Tests with cargo-nextest

CI runs the unit-test stage with [cargo-nextest](https://nexte.st) (issue #1223).
Nextest runs every test in its own process, in parallel across binaries, and
produces JUnit output that CI uses for partitioning and flaky-test reporting.

## Install

```bash
cargo install cargo-nextest --version 0.9.87 --locked
# or a prebuilt binary:
curl -LsSf https://get.nexte.st/0.9.87/linux | tar zxf - -C ~/.cargo/bin
```

`cargo test` keeps working; nextest is faster for the full suite and gives
better subset selection.

## Running subsets locally

```bash
# Whole workspace (local `default` profile: fail fast, no retries)
cargo nextest run --workspace

# One crate
cargo nextest run -p ledgerlens-score

# Tests whose name contains a substring
cargo nextest run -p ledgerlens-score heartbeat

# One test module, using a filterset expression
cargo nextest run -p ledgerlens-score -E 'test(/^test_governance_chain::/)'

# Everything except slow adversarial suites
cargo nextest run --workspace -E 'not test(/adversarial/)'

# Reproduce one CI partition exactly (see "Partitioning" below)
cargo nextest list --workspace --message-format json > target/list.json
python3 tools/ci/partition_tests.py --list target/list.json --shards 4 --index 2 > target/filter.txt
cargo nextest run --workspace -E "$(cat target/filter.txt)"

# Use the CI or nightly profile
cargo nextest run --workspace --profile ci
```

Filterset syntax is documented at <https://nexte.st/docs/filtersets/>.

## Profiles (`.config/nextest.toml`)

| Profile | Used by | Timeouts | Retries | Output |
|---|---|---|---|---|
| `default` | local runs | slow after 30s, killed after 2 min | none | failures only, fail fast |
| `ci` | PR / main CI | slow after 60s, killed after 5 min | none | JUnit at `target/nextest/ci/junit.xml`, no fail fast |
| `nightly` | `nightly-tests.yml` | slow after 120s, killed after 20 min | none | JUnit with full output |

**Retries are allowed only for network-dependent tests**, identified by a test
name segment starting with `network_` (for example `mod::network_fetch_snapshot`).
No other test is ever retried. If you add a test that genuinely needs the
network, name it accordingly; deterministic tests must not use that prefix.

## Flaky and slow test reporting

A test that fails and then passes on retry is recorded by nextest as a
`<flakyFailure>` in the JUnit report. The `Flaky and slow test report` CI job
(`tools/ci/nextest_report.py --fail-on-flaky`) lists every such test, emits a
warning annotation and **fails**, so a retry-pass is reported as flaky, never
as green. The job also lists the slowest tests and uploads `test-report.json`
(retained 90 days) for trend tracking; the nightly workflow does the same for
the full suite.

## Partitioning

The `Unit tests` job runs as four parallel partitions. Each partition:

1. downloads the JUnit reports of the latest successful run on `main`;
2. runs `tools/ci/partition_tests.py`, which assigns every current test to a
   shard with a longest-processing-time-first greedy algorithm using those
   historical durations (tests with no history are weighted with the median);
3. runs only its shard via a nextest filterset.

Every listed test lands in exactly one shard, so new tests are never skipped.
When no history is available (first run, expired artifacts) the split degrades
to an even count-based split.

## Doctests and benches

Nextest does not run doctests. The `Doctests and benches` CI job runs
`cargo test --workspace --doc` and `cargo test --workspace --benches`, which
executes each Criterion benchmark once in test mode so benches keep compiling
and running. Full benchmark measurements run nightly in `nightly-tests.yml`.

## Measured impact

Baseline (`cargo test --workspace` single job on `main`, last three runs
before this change): 12.2 min, 10.9 min, 13.2 min, median **12.2 min**. The
partitioned stage's wall clock is recorded in the PR that introduced it and
tracked afterwards by the `CI metrics` workflow ([ci-performance.md](ci-performance.md)).
