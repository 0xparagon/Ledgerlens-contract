# Nightly fuzzing and soak

Workflow: [`.github/workflows/nightly-fuzz.yml`](../.github/workflows/nightly-fuzz.yml) (issue #1226).

## What runs

Every night at 02:00 UTC, or on manual dispatch with a custom `budget_minutes` value (default 90):

1. **Decoder / regression targets:** each fixture in the persisted corpus is replayed
   with `invocation-fuzzer replay`, which decodes the campaign and checks that replay is deterministic.
2. **Soak campaigns:** seeded `invocation-fuzzer smoke --cases 512` campaigns run back to back,
   each with a new seed (`YYYYMMDD * 1000 + n`), until the time budget runs out.

## Corpus persistence and minimisation

`fuzz-state/` is kept between runs with `actions/cache` (a new key per run and a prefix restore).
Before each run, the checked-in `tools/invocation-fuzzer/corpus/*.json` fixtures are merged in.
Every file is canonicalised with `jq -S` and renamed to its content hash, which removes duplicates.
Each run appends one line to `fuzz-state/history.jsonl`. The job summary shows the last 14 runs:
number of campaigns, corpus size, and peak behaviour signatures, which serve as the coverage measure.

## Crash triage

When a run fails:

- A reproducer bundle (`crash/`: log, corpus snapshot, `REPRODUCE.txt`) is uploaded as the
  `fuzz-crash-<sig>` artifact.
- `<sig>` is a hash of the first failing line with numbers and hex values removed. Repeated crashes
  therefore map to one open issue titled `fuzz crash [<sig>]`: a new crash opens that issue, and a repeat adds a comment to it.
- Crashes whose log mentions read entry points (`get_score`, `read_*`, query/view) or security
  oracles/invariants also get the `severity:high` label.

To reproduce, download the artifact into the repository root and run the command in `REPRODUCE.txt`.

## Security

The job needs only `contents: read` and `issues: write`. It is guarded by
`github.repository == 'Ledger-Lenz/Ledgerlens-contract'`, so it never runs on forks. It has no
`pull_request` trigger, so untrusted code never runs with these permissions.
