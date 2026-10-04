# Replay Harness

A deterministic replay tool for regression testing the LedgerLens contract with real Stellar mainnet trade history.

## Overview

The replay harness reads a Horizon API snapshot (NDJSON format) and feeds it through the off-chain detection pipeline simulation and contract submission functions. It validates:

- **No panics**: all contract calls complete without panicking
- **Score range**: all accepted scores remain in [0, 100]
- **Rate limits**: repeated submissions for the same (wallet, pair) are rate-limited as expected
- **Numerical stability**: no arithmetic overflow in aggregate score computations
- **Determinism**: identical input produces identical contract state and call sequences

## Snapshot Format

Each line is JSON:
```json
{"wallet":"wallet_id","asset_pair":"XLM_USDC","trades":[{"price":0.12},{"price":0.13}]}
```

- `wallet` (string): Horizon account ID or identifier
- `asset_pair` (string): asset pair symbol (e.g., "XLM_USDC")
- `trades` (array, optional): trade records with `price` field

## Native forked-state loader

The replay crate ships a native loader that builds a replay starting point
directly from live testnet state over RPC, replacing the deprecated Python
snapshot script (`scripts/fetch_testnet_snapshot.py`). It shares the canonical
replay types with the rest of the toolchain and verifies every fetched entry
before it is written to the snapshot.

### Fetching a snapshot

```bash
cargo run -p replay --manifest-path tools/replay/Cargo.toml -- \
  fetch-snapshot --rpc-url <RPC_URL> --output snapshot.ndjson
```

Options:

- `--rpc-url <URL>`: Soroban RPC endpoint to fetch ledger entries from.
- `--output <PATH>`: destination for the canonical replay snapshot.
- `--key-family <FAMILY>`: restrict fetching to a key family (repeatable).
- `--max-entries <N>`: upper bound on the number of entries fetched.
- `--max-bytes <N>`: upper bound on the total serialized entry size.
- `--include-ttl`: also fetch and record TTL data for the selected entries.
- `--page-size <N>`: entries requested per RPC page (defaults to the RPC cap).
- `--max-retries <N>`: retry attempts for transient RPC failures.
- `--rate-limit <N>`: maximum requests per second, to stay within RPC limits.

### Paging, retry and rate limiting

Entries are fetched page by page using the RPC cursor. Each page is retried on
transient failures (timeouts, 429s, 5xx) with bounded backoff, and requests are
throttled to the configured rate limit. Partial pages are handled by continuing
from the returned cursor; a page that cannot be completed within the retry
budget aborts the fetch with a descriptive error rather than emitting a
truncated snapshot.

### Verification and attribution

Every fetched entry is verified before being written:

- the contract id is checked against the requested key family, and
- the ledger key is decoded through the shared replay types.

Entries that fail verification are rejected and reported. The snapshot records
the ledger sequence and ledger hash the entries were fetched at, so a snapshot
is reproducible and attributable to a specific point in chain history.

### Parity and failure-mode tests

Parity tests run the native loader and the deprecated Python script against a
recorded RPC fixture server and assert that both produce identical snapshots.
Failure-mode tests cover timeouts, partial pages and malformed responses. The
Python script is kept as deprecated until parity tests pass, after which it is
removed.

## Building

```bash
cargo build -p replay
```

## Running

```bash
cargo run -p replay --manifest-path tools/replay/Cargo.toml
```

By default, the binary reads `testdata/reference.ndjson` relative to the replay
crate, so it works when launched from either the workspace root or this directory.
An explicit snapshot path is still interpreted relative to the caller's current
working directory.

### Configuration drift detection

The same binary can compare an approved deployment manifest against an
observed live snapshot:

```bash
cargo run -p replay --manifest-path tools/replay/Cargo.toml -- \
  config-drift approved.json observed.json
```

To emit a template containing the supported config surface:

```bash
cargo run -p replay --manifest-path tools/replay/Cargo.toml -- \
  config-drift --template
```

Diff output is deterministic JSON sorted by field name. Unknown approved
fields, unknown observed fields, missing observed fields, and value drift are
reported separately so operators can distinguish manifest mistakes from
unauthorized on-chain changes.

## Testing

Integration tests verify replay logic and contract call safety:

```bash
cargo test -p replay
```

## Sample Data

`testdata/reference.ndjson` contains 25 entries across 20 wallets and 5 asset pairs, exercising:
- Empty trade lists
- Single and multi-trade entries
- Repeated wallets across multiple pairs
- Realistic price ranges

## CI Integration

The workflow `.github/workflows/replay-regression.yml` runs the replay harness on every PR to detect regressions in score submissions or contract behavior.
