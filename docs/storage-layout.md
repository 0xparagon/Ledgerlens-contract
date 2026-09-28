# Storage Layout and Rent Mechanics

This document provides a comprehensive specification of the on-chain storage structure, keys, storage tiers, Time-to-Live (TTL) settings, and rent mechanics utilized in the LedgerLens smart contract.

---

## Soroban Storage Tiers & Rent Mechanics

The Stellar Soroban platform implements a state-archiving rent model. Smart contracts and their associated data consume ledger space, which incurs rent fees. Rent is determined by the storage footprint size (in bytes) and the duration (in ledgers) the entry resides on the network.

To manage rent efficiently, Soroban offers three distinct storage tiers, each with unique pricing, lifecycle properties, and restoration paths:

1. **Instance Storage**
   - **Characteristics**: Shared storage bound directly to the contract instance. Contains contract configurations, administration state, and global constants. 
   - **Behavior**: Loaded automatically whenever any function in the contract is called. Accessing instance storage has a higher initial gas charge but requires no separate key lookup gas or footprint declaration.
   - **TTL Lifecycle**: Shared directly with the contract instance itself. It does not have an independent `extend_ttl` invocation in standard storage helper paths; its lifetime is extended automatically whenever the contract is executed or upgraded.

2. **Persistent Storage**
   - **Characteristics**: Stored under individual keys separate from the contract instance. Best suited for user data, transaction records, and variables that must remain intact across upgrades.
   - **Behavior**: Loaded on demand. Accessing a persistent key requires declaring it in the transaction footprint.
   - **TTL Lifecycle**: If the TTL (Time-To-Live) of a persistent entry expires, the entry is **archived** (moved to cold storage). Archived data can be restored by submitting a restoration transaction and paying a recovery fee.

3. **Temporary Storage**
   - **Characteristics**: The least expensive tier, designed for ephemeral state such as rate limits, cooldowns, or time-locked flags.
   - **Behavior**: Loaded on demand. Requires footprint declaration.
   - **TTL Lifecycle**: Once a temporary entry's TTL expires, it is **permanently deleted** and cannot be restored. To re-establish the state, the key must be written anew.

---

## TTL Extension Triggers (Read vs. Write Paths)

LedgerLens dynamically extends the TTL of active keys to prevent unexpected archiving or expiration. However, how and when these extensions are triggered is critical to gas consumption:

### Write Paths
When writing or updating a key (e.g., `set_score`), the contract always calls `extend_ttl(&key, threshold, extend_to)` immediately following the write. This resets the entry's lifetime on the ledger to the target maximum (`extend_to`) if its remaining lifetime falls below `threshold`.

### Read Paths
When reading a key via standard getters (e.g., `get_score`), the contract checks if the key exists. If it is present, it calls `extend_ttl` to refresh the entry's lifetime. This keeps active entries alive indefinitely as long as they are regularly accessed.

### "Peek" / Read-only Paths (Side-effect-free Queries)
Soroban treats TTL extension as a **state-mutating write operation**. Consequently, any transaction invoking a function that extends a TTL cannot be run in a read-only context, and triggers state fees.

To allow gas-free and infallible integrations from external smart contracts (e.g., AMM guards calling `query_risk_gate`), LedgerLens provides separate **peek** read paths (e.g., `peek_score`, `peek_risk_band_state`, `peek_is_embargoed`). These functions perform direct reads without calling `extend_ttl`. 
> [!NOTE]
> Integrating contracts should always call the `peek` versions or view methods that do not trigger TTL extensions to avoid adding unnecessary gas overhead and write footprint requirements to their query paths.

---

## Archival-Aware Reads and Restore Footprint

Persistent entries whose TTL lapses are **archived**: they are removed from the live ledger and moved to cold storage. A transaction that reads or writes an archived key fails unless the same transaction first restores the entry (via a Soroban restore operation) and declares the key in its footprint. This section documents exactly how each LedgerLens entry point behaves when its entries are archived, and how off-chain tooling computes the restore footprint ahead of submission.

### Entry Point vs. Archived-Key Behavior Matrix

| Entry Point | Storage Tier | Behavior on Archived Key | Consumer Guidance |
| :--- | :--- | :--- | :--- |
| `set_score` (write) | Persistent | **Fails closed.** The write cannot proceed against an archived entry; the transaction aborts unless the key is restored first. | Restore the score key before submitting a write. |
| `get_score` (read, extends TTL) | Persistent | **Fails closed.** The read aborts on an archived key; it never returns a default. | Restore before reading if a live value is required. |
| `peek_score` (read-only) | Persistent | **Fails closed.** Returns no value for an archived key; it does **not** synthesize `0` or "no score". | Treat a failed/absent peek as *unknown*, never as *safe*. |
| `query_risk_gate` (gate) | Persistent | **Fails closed.** An archived score entry causes the gate query to fail rather than pass. | A gate must never interpret archival as "no score, therefore safe". |
| `peek_risk_band_state` (read-only) | Persistent | **Fails closed.** No synthesized band for an archived entry. | Fail the integration path; do not default to a permissive band. |
| `peek_is_embargoed` (read-only) | Persistent | **Fails closed.** No synthesized embargo state for an archived entry. | Fail closed; do not assume "not embargoed". |
| Instance-key reads (e.g. `RiskThreshold`, `Paused`) | Instance | **Not applicable.** Instance storage shares the contract instance TTL and is loaded automatically; it is not independently archived. | No restore footprint required for instance keys. |
| Temporary-key reads (e.g. cooldown flags) | Temporary | **Permanently deleted** on TTL lapse; cannot be restored. | Re-establish state by writing the key anew. |

> [!IMPORTANT]
> **Fail-closed rule.** An archived persistent entry must never be mistaken for "no score, therefore safe". Every read, peek, and gate path above fails closed on an archived key. Consumers must treat a failed or absent result as *unknown* and deny by default, restoring the entry and retrying only when a live value is genuinely required.

### Restore Footprint Helper

Before submitting a transaction that touches a wallet's score (or a set of wallet/pair keys), off-chain tooling must compute the **restore footprint**: the exact set of persistent ledger keys that need to be restored. The helper lives in the recovery tooling (`tools/recovery`) and is unit-tested.

Given a set of wallets (and optional asset pairs), the helper:

1. Derives the persistent storage keys LedgerLens uses for each wallet/pair (e.g. the score key and any per-pair keys).
2. Filters to keys that are currently archived (TTL lapsed) by consulting the ledger entry state.
3. Returns the deduplicated, deterministically ordered list of keys to include in the restore operation's footprint.

```text
restore_footprint(wallets, pairs) -> [LedgerKey]
  keys = []
  for wallet in wallets:
    keys += score_key(wallet)
    for pair in pairs:
      keys += pair_key(wallet, pair)
  keys = dedupe(keys)
  return [k for k in keys if is_archived(k)]
```

The footprint computation is covered by unit tests that assert: (a) only archived keys are returned, (b) live keys are excluded, (c) duplicate wallet/pair inputs are deduplicated, and (d) ordering is deterministic so the resulting footprint is stable across runs.

### Tests: Simulating Archival and Restoration

The test harness models archival by advancing the ledger sequence past a persistent entry's TTL, then asserting the documented behavior:

- **Archived read fails closed**: after a score entry is archived, `get_score` and `peek_score` fail rather than returning a default.
- **Archived gate fails closed**: `query_risk_gate` on an archived entry fails; it is never treated as "no score, therefore safe".
- **Restore then read succeeds**: restoring the archived key and re-running the read returns the original value.
- **Footprint helper**: the restore-footprint computation returns exactly the archived keys for the given wallet/pair set, with live keys excluded and duplicates removed.

---

## Storage Layout Specifications

The following tables specify every key stored by LedgerLens, mapped to its storage tier, TTL parameters, and purpose.

### Instance Storage Keys
*Instance storage keys do not have independent TTL properties; they inherit the contract instance's TTL.*

| Key Name | Storage Tier | TTL Threshold | TTL Extend-To | Description | Cross-Reference |
| :--- | :--- | :---: | :---: | :--- | :--- |
| `Admin` | Instance | N/A | N/A | The contract administrator address. | - |
| `AdminSet` | Instance | N/A | N/A | The list of multi-sig admin co-signers. | [`MAX_ADMIN_SIGNERS`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L92) |
| `AdminThreshold` | Instance | N/A | N/A | The required number of co-signatures for administrative commands. | - |
| `Service` | Instance | N/A | N/A | The address of the primary off-chain scoring service (single-signer path). | - |
| `ServiceSet` | Instance | N/A | N/A | The set of addresses authorized to co-sign score submissions. | [`MAX_SERVICE_SIGNERS`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L89) |
| `ServiceThreshold` | Instance | N/A | N/A | The required signature count for M-of-N consensus. | - |
| `ServicePubKey` | Instance | N/A | N/A | The off-chain pipeline's secp256k1 public key used to verify ECDSA signatures. | - |
| `SignerTier(Address)` | Instance | N/A | N/A | The authorized score range limits (`TierBounds`) for service signers. Defaults to `[0, 100]` if unset. | - |
| `Paused` | Instance | N/A | N/A | Global boolean pause switch. | - |
| `PendingAdmin` | Instance | N/A | N/A | Pending new admin address during administrative handovers. | - |
| `RiskThreshold` | Instance | N/A | N/A | Global threshold above which scores trigger breach events. | [`DEFAULT_RISK_THRESHOLD`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L16) |
| `JumpThreshold` | Instance | N/A | N/A | Absolute delta limit between consecutive scores that triggers anomaly events. | `DEFAULT_JUMP_THRESHOLD` |
| `HistoryMaxDepth` | Instance | N/A | N/A | Depth of the `ScoreHistory` ring buffer. | [`DEFAULT_HISTORY_MAX_DEPTH`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L10), [`MAX_HISTORY_DEPTH`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L7) |
| `ContractVersion` | Instance | N/A | N/A | Semantic contract version. | [`CONTRACT_VERSION`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L29) |
| `PendingUpgrade` | Instance | N/A | N/A | Current locked-in `UpgradeProposal` for contract WASM upgrades. | - |
| `UpgradeDelay` | Instance | N/A | N/A | Delay in seconds between proposal and execution of WASM upgrades. | [`DEFAULT_UPGRADE_DELAY_SECS`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L86) |
| `StalenessWindow` | Instance | N/A | N/A | Maximum age in seconds before a score is considered stale. | [`DEFAULT_STALENESS_WINDOW_SECS`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L95) |
| `CooldownSecs` | Instance | N/A | N/A | Rate limit cooldown delay between submissions for the same key. | [`DEFAULT_COOLDOWN_SECS`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L57) |
| `DecayRateNumerator` | Instance | N/A | N/A | Fixed-point exponential decay numerator λ. Defaults to 0. | [`DEFAULT_DECAY_LAMBDA_NUM`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L113) |
| `DecayRateDenominator` | Instance | N/A | N/A | Fixed-point exponential decay denominator λ. Defaults to 1. | [`DEFAULT_DECAY_LAMBDA_DEN`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.r