# Configuration drift detection

LedgerLens now defines a stable operator-facing configuration manifest for
deployed `ledgerlens-score` instances and ships a deterministic drift checker
through the `replay` tool.

## Stable manifest fields

- `contract_version`
- `paused`
- `risk_threshold`
- `jump_threshold`
- `staleness_window`
- `upgrade_delay`
- `cooldown`
- `service_threshold`
- `admin_threshold`
- `consensus_threshold_k`
- `consensus_epsilon`
- `reveal_window`
- `finality_buffer`
- `heartbeat_alert_threshold`
- `oracle_staleness_threshold`

## Workflow

1. Capture the approved manifest in JSON.
2. Query the live deployment and materialize the same JSON object.
3. Run:

```bash
cargo run -p replay --manifest-path tools/replay/Cargo.toml -- \
  config-drift approved.json observed.json
```yaml

4. Treat any `drift`, `missing_observed_field`, `unexpected_observed_field`,
   `unknown_approved_field`, or `unknown_observed_field` entry as an operator
   review item.

## Provenance index

Every governed parameter now carries an on-chain provenance record that is
written atomically inside the shared setter path, so it can never diverge from
the stored value. Each record is compact and contains:

- `last_changed_ledger` — ledger sequence of the most recent change.
- `action_id` — the governance action (or proposal) id that authorized it.
- `actor_set_digest` — digest of the actor set that signed the change.
- `previous_value_digest` — digest of the value that was replaced.

### Reading provenance

The paginated read `get_parameter_provenance(cursor, limit)` returns parameters
with their provenance, ordered by the same stable field order used by the export
manifest above. The response is aligned with the configuration export format so
the drift checker can consume it directly: each entry exposes the parameter
name, its current value, and the provenance record fields.

### Drift detection with provenance

When comparing an approved manifest against a live deployment, the drift checker
can additionally surface provenance for any field that differs. A changed value
whose `previous_value_digest` does not match the approved value's digest, or
whose `action_id` is not in the approved change set, is reported as an
unexpected change rather than a plain `drift` entry. This lets operators
distinguish an authorized configuration change from silent drift.

## Storage and cost

- Provenance is stored in a single bounded map keyed by parameter, so storage
growth is bounded by the fixed number of governed parameters.
- Each setter writes exactly one provenance record in addition to the value
  update. The documented write-cost increment is one map entry write per setter
  call (one additional storage slot), keeping the per-setter cost bounded and
  predictable.

## Compatibility notes

- No on-chain storage layout changed for existing values; provenance is stored
  in a new bounded map.
- No contract ABI or event changed.
- The drift checker is off-chain only and reads JSON snapshots.
- The supported manifest field list is exported from the Rust crate so the
  tool and contract documentation stay aligned.
