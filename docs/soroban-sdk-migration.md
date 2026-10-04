# soroban-sdk migration plan: 21.x → 22.x → 23.x

Tracking issue: #1228. Status: **planned, not yet executed.** The workspace still pins `soroban-sdk = "21.0.0"`.
This document gives the impact analysis and the staged procedure. The upgrade itself, the gate deltas and
the testnet rehearsal must be recorded here in the upgrade PR before #1228 is closed.

## Impact analysis by major

| Area | 21 → 22 (protocol 22) | 22 → 23 (protocol 23) |
|---|---|---|
| **ABI** | `Env::register` / `register_at` replace `register_contract` / `register_contract_wasm`. Contract constructors (`__constructor`) are supported. The exported function set is unchanged unless we adopt constructors. | `#[contractevent]` adds event specs to the contract spec. The spec/metadata sections grow. Exported functions are unchanged. |
| **Storage** | No change to key encoding or durability semantics. TTL/extend APIs are unchanged. | Storage key encoding is unchanged. The archival/restore behaviour of protocol 23 affects persistent entries that have been evicted, so check the rent assumptions in `docs/rent-griefing-analysis.md`. |
| **Authorisation** | `require_auth` semantics are unchanged. Testutils `mock_all_auths` is unchanged. | `require_auth` semantics are unchanged. |
| **Events** | `events().all()` return types are unchanged. | Typed events via `#[contractevent]`. The legacy `events().publish` is deprecated but still emits identical XDR. Keep the legacy emitters until `docs/EVENT_SCHEMA_STABILITY.md` allows a change. |
| **Testutils** | Test snapshot JSON format changes, so `test_snapshots/` must be regenerated. Deprecation warnings for `register_contract` break `-D warnings`. | Snapshot format and budget accounting change again, so regenerate snapshots and re-baseline the budget tests. |
| **Costs** | CPU/memory cost model is recalibrated. Re-baseline `docs/resource-budgets.md`. | Cost model is recalibrated again. Re-baseline. |
| **Toolchain** | Still `wasm32-unknown-unknown` with Rust ≤ 1.81. | Requires a newer Rust. Build with the `wasm32v1-none` target (Rust ≥ 1.84), which lifts the 1.82 restriction described in the host policy. |

## Deployed instances

On-chain behaviour depends on the network protocol version, not on the SDK version the WASM was built with.
Existing instances keep running their current WASM under the network's host until `upgrade` installs the new WASM.
After an upgrade the new WASM uses the same storage keys, so behaviour changes are limited to the SDK-level
differences listed above. Every difference must be confirmed with the replay regression corpus (`tools/replay`).

## Procedure (per major, on a dedicated branch)

1. Bump every `soroban-sdk` requirement in lockstep, then run `cargo update -p soroban-sdk`.
2. Run the full matrix: `ci.yml`, the ABI export diff (`sdk-compat.yml`), the WASM size gate, and `replay-regression.yml`.
   Record each delta in the table below.
3. Regenerate `test_snapshots/` and re-baseline the budget tests in a separate commit.
4. Testnet rehearsal: run `scripts/rehearsal.sh` to deploy the **old** WASM and populate representative state,
   then upgrade to the new WASM and run `scripts/verify-deployment.sh` and the replay corpus against the upgraded instance.
   Record the transaction hashes here.
5. Update `rust-toolchain.toml`, [`host-version-support-policy.md`](host-version-support-policy.md) and
   [`reproducible-builds.md`](reproducible-builds.md). Add the last SDK-21 WASM to `tests/fixtures/historical/`
   so that old-SDK compatibility stays covered by `docs/historical-wasm-compatibility.md`.

## Recorded deltas

| Step | ABI diff | WASM size | CPU budget | Replay corpus | Approved by |
|---|---|---|---|---|---|
| 21 → 22 | _pending_ | _pending_ | _pending_ | _pending_ | |
| 22 → 23 | _pending_ | _pending_ | _pending_ | _pending_ | |

## Testnet rehearsal log

_Pending: record the contract id, the upgrade transaction and the verification output here._
