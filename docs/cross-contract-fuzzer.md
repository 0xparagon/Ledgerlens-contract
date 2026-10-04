# Cross-contract invocation fuzzer

The `invocation-fuzzer` workspace tool exercises the real Soroban
cross-contract paths from the mock AMM, mock lending market, and LedgerLens
aggregator into `ledgerlens-score`. It is deterministic, bounded, and intended
for both pull-request smoke coverage and longer local campaigns.

## Security invariant and oracle

For every generated operation sequence:

1. Replaying the same versioned JSON campaign from a fresh environment produces
   the same ordered operation outcomes and final score fingerprint.
2. A gate query, rejected payload, malformed invocation, or unavailable
   dependency cannot modify the observed LedgerLens score or emit a contract
   event. Score changes are permitted only for an explicit `submit_score`
   setup operation.
3. Integrators fail closed for missing, unsafe, low-confidence, malformed, and
   unavailable oracle inputs.
4. Execution is bounded before any Soroban value is allocated.
5. The contract's existing testutils invariant registry is evaluated after
  every operation, and panics from supported read-entry points fail the run.

The smallest observable state is the score, confidence, timestamp, and model
version for one fixed wallet/pair, plus the ordered outcome category and
contract event count for each invocation. This deliberately avoids snapshots of
unrelated storage and keeps replay fixtures stable across compatible internal
refactors.

## Trust and authorization boundaries

The fuzzer is native test tooling, not deployed WASM. Its setup phase uses
Soroban test authorization to initialize contracts and submit scores. The
targeted public gate methods are read-only and require no caller
authorization; calls made by the AMM, lending, and aggregator contracts
exercise the contract-as-caller boundary. No production authorization policy,
public type, event, error discriminant, or storage key is changed.

## Run and replay

Run the same bounded campaign used by CI:

```bash
cargo run -p invocation-fuzzer --locked -- smoke --seed 1666232542 --cases 32
```yaml

Run a larger local campaign, up to the hard maximum of 512 generated cases:

```bash
cargo run -p invocation-fuzzer --locked -- smoke --seed 42 --cases 512
```

Generate the score contract ABI input schema from the built contract spec and
use it to add structure-aware read-entry calls to the campaign:

```bash
cargo run -p schema-gen -- --wasm target/wasm32-unknown-unknown/release/ledgerlens_score.wasm \
  --invocations --out target/invocation-fuzzer
cargo run -p invocation-fuzzer --locked -- smoke --seed 42 --cases 512 \
  --abi-schema target/invocation-fuzzer/ledgerlens_score.invocations.schema.json
```

Replay one persisted regression exactly twice:

```bash
cargo run -p invocation-fuzzer --locked -- replay \
  tools/invocation-fuzzer/corpus/003-boundaries-and-malformed.json
```

Corpus files are loaded in lexicographic order. The PR smoke seed, mutation
selection, fresh-environment setup, and replay comparison are deterministic.
The tool retains an input in its in-memory mutation queue only when it discovers
a new `(operation kind, observable outcome)` behavior signature. ABI-driven
generation selects read-only `get_*`, `query_*`, `is_*`, `peek_*`, and
`supports_*` functions directly from `contractspecv0`; numeric ranges come from
the schema, and collection generation samples empty, singleton, duplicate, and
one-over-budget values. Contract initialization and mock dependency setup are
performed before each campaign, so stateful submissions run only after those
preconditions hold.

## Failure shrinking and regression fixtures

When a generated case violates an invariant or is non-deterministic, the tool:

1. removes operations while the failure remains;
2. simplifies the remaining numeric, symbol, and raw-argument values;
3. writes the minimized JSON to
   `target/invocation-fuzzer/failures/seed-<seed>.json`; and
4. reports the exact `replay` command to use.

After diagnosis, copy every confirmed counterexample into
`tools/invocation-fuzzer/corpus/` with a descriptive, ordered filename and
commit it with the fix. CI replays every committed fixture before mutations, so
regressions cannot be silently skipped.

New behavior signatures also produce minimized seed files under
`target/invocation-fuzzer/corpus/`. Promote confirmed cases to the committed
corpus so they become durable regression fixtures. Read-entry panics fail the
campaign and are saved through the same minimized-failure path.

## Explicit resource bounds

| Resource | Hard bound or signal |
|---|---|
| Generated cases | 512 per process |
| Operations | 16 per campaign |
| Raw arguments | 8 per invocation |
| Soroban symbol | 1-32 ASCII alphanumeric/underscore bytes |
| Wire string | 64 bytes |
| Encoded campaign | 16 KiB |
| Score oracle reads | At most `2 * operations + 1` |
| Score writes | At most one attempted setup write per `submit_score` operation |
| Read-only events | Exactly zero; otherwise the campaign fails |

The final `MAX` line reports the highest observed CPU instruction cost, memory
cost, encoded bytes, operation/argument counts, logical score reads/writes, and
contract event count across the run. These signals are stable enough for CI
review while the hard limits prevent an input from turning the harness into an
unbounded resource consumer.

## Monitoring, diagnosis, and recovery

- `harness_rejected:` is an input-shape rejection by the bounded harness, such
  as an empty or overlong symbol. It is an expected fuzz outcome.
- A Soroban `Err(...)` outcome is a contained contract rejection or trap. Check
  the preceding operation and replay the fixture.
- `invariant violation` means a read-only path changed score state or emitted an
  event. Treat this as a security regression.
- `deterministic replay mismatch` means the same campaign produced different
  observable results from fresh environments.

The tool never writes contract state outside its in-memory `Env`. Recovery is
therefore operational: stop the campaign, replay the minimized file, fix the
target, persist the fixture, and rerun the smoke command. Rollback consists of
reverting the tooling change; no on-chain migration or downgrade action is
required.

## Design alternatives rejected

- **Unbounded libFuzzer input:** rejected because CI replay, resource limits,
  and human-readable regression fixtures are required.
- **Line-coverage-only guidance:** rejected because it does not prove the
  fail-closed and read-only invariants. Behavior signatures guide this corpus.
- **Production contract hooks:** rejected because fuzz observability must not
  expand the deployed ABI or storage layout.
- **Random host entropy:** rejected because it prevents deterministic replay and
  stable shrinking.
