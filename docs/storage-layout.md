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

## Instance-Storage Footprint Audit

Instance storage is loaded on **every** invocation of the contract, so any growth there taxes every entry point, including cheap reads. This section inventories every key that currently lives in instance storage, estimates its serialized size, and records how often it is read on the hot paths. It is the source of truth for the instance-size budget enforced by the regression test in `contracts/ledgerlens-score/src/storage.rs`.

### Size estimation method

Sizes are estimated from the XDR encoding of each value type (addresses are 32-byte account/contract identifiers plus a discriminant; `u32`/`u64`/`i128` are fixed-width; `Vec<Address>` is `4 + n * 32`; `BytesN<33>` is 33 bytes). Estimates are conservative upper bounds and are used only to compare relative weight and to set the budget, not as exact ledger bytes.

### Inventory of instance-storage keys

| Key Name | Storage Tier | Est. Size (bytes) | Access Frequency | Description | Cross-Reference |
| :--- | :--- | :---: | :--- | :--- | :--- |
| `Admin` | Instance | ~36 | Hot (auth on every admin call) | The contract administrator address. | - |
| `AdminSet` | Instance | ~4 + n*32 | Cold (admin rotation only) | The list of multi-sig admin co-signers. | [`MAX_ADMIN_SIGNERS`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L92) |
| `AdminThreshold` | Instance | ~4 | Cold (admin rotation only) | The required number of co-signatures for administrative commands. | - |
| `Service` | Instance | ~36 | Warm (score submission auth) | The address of the primary off-chain scoring service (single-signer path). | - |
| `ServiceSet` | Instance | ~4 + n*32 | Warm (score submission auth) | The set of addresses authorized to co-sign score submissions. | [`MAX_SERVICE_SIGNERS`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L89) |
| `ServiceThreshold` | Instance | ~4 | Warm (score submission auth) | The required signature count for M-of-N consensus. | - |
| `ServicePubKey` | Instance | ~33 | Warm (ECDSA verification on submit) | The off-chain pipeline's secp256k1 public key used to verify ECDSA signatures. | - |
| `SignerTier(Address)` | Instance | ~4 + n*8 | Cold (per-signer config) | The authorized score range limits (`TierBounds`) for service signers. Defaults to `[0, 100]` if unset. | - |
| `Paused` | Instance | ~1 | Hot (checked on every mutating call) | Global boolean pause switch. | - |
| `PendingAdmin` | Instance | ~36 | Cold (handover only) | Pending new admin address during administrative handovers. | - |
| `RiskThreshold` | Instance | ~4 | Hot (breach check on every score) | Global threshold above which scores trigger breach events. | [`DEFAULT_RISK_THRESHOLD`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L16) |
| `JumpThreshold` | Instance | ~4 | Hot (anomaly check on every score) | Absolute delta limit between consecutive scores that triggers anomaly events. | `DEFAULT_JUMP_THRESHOLD` |
| `HistoryMaxDepth` | Instance | ~4 | Warm (history writes) | Depth of the `ScoreHistory` ring buffer. | [`DEFAULT_HISTORY_MAX_DEPTH`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L10), [`MAX_HISTORY_DEPTH`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L7) |
| `ContractVersion` | Instance | ~4 | Cold (upgrade/version queries) | Semantic contract version. | [`CONTRACT_VERSION`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L29) |
| `PendingUpgrade` | Instance | ~80 | Cold (upgrade flow only) | Current locked-in `UpgradeProposal` for contract WASM upgrades. | - |
| `UpgradeDelay` | Instance | ~8 | Cold (upgrade flow only) | Delay in seconds between proposal and execution of WASM upgrades. | [`DEFAULT_UPGRADE_DELAY_SECS`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L86) |
| `StalenessWindow` | Instance | ~8 | Warm (staleness check on reads) | Maximum age in seconds before a score is considered stale. | [`DEFAULT_STALENESS_WINDOW_SECS`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L95) |
| `CooldownSecs` | Instance | ~8 | Warm (rate-limit check on submit) | Rate limit cooldown delay between submissions for the same key. | [`DEFAULT_COOLDOWN_SECS`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L57) |
| `DecayRateNumerator` | Instance | ~16 | Warm (decay math on reads) | Fixed-point exponential decay numerator λ. Defaults to 0. | [`DEFAULT_DECAY_LAMBDA_NUM`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L113) |
| `DecayRateDenominator` | Instance | ~16 | Warm (decay math on reads) | Fixed-point exponential decay denominator λ. Defaults to 1. | [`DEFAULT_DECAY_LAMBDA_DEN`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L113) |

### Instance-size budget

The instance footprint is dominated by the fixed-size configuration keys above. The regression test `test_instance_storage_size_budget` in `contracts/ledgerlens-score/src/storage.rs` measures the serialized size of the instance map after a representative configuration is written and fails if it exceeds the documented budget:

- **Budget:** `MAX_INSTANCE_STORAGE_BYTES` (see `contracts/ledgerlens-score/src/constants.rs`).
- **Rationale:** the budget is set with headroom above the current measured footprint so that adding a new instance key requires an explicit, reviewed bump rather than silently taxing every entry point.

### Recommendations

- **Keep hot keys in instance storage.** `Admin`, `Paused`, `RiskThreshold`, `JumpThreshold`, and the service-auth keys are read on nearly every call; moving them to persistent storage would add a footprint declaration and a key lookup to the hottest paths and is not warranted.
- **Move cold configuration to persistent storage where safe.** `PendingUpgrade`, `UpgradeDelay`, `ContractVersion`, `PendingAdmin`, and `AdminSet`/`AdminThreshold` are read only during rare administrative flows. They are candidates for persistent storage in a future change.
- **Migration plan (for the future move).** A migration entry point would read each cold key from instance storage, write it to persistent storage under the same `DataKey` variant, and remove the instance entry. Because the `DataKey` enum is unchanged, the ABI is unaffected; only the storage tier changes. The migration must be idempotent and gated behind the admin auth path.
- **ABI impact statement.** No public function signature, event, or error enum changes are required by this audit. The inventory and budget are documentation plus a test; any tier move would be a separate, explicitly reviewed change.

### Benchmark note

No storage tier was changed by this audit, so no CPU/memory improvement is claimed. The regression test pins the current footprint so that future growth is caught before it can regress the hottest read entry points (`peek_score`, `get_score`). If a tier move is later performed, the same test plus the existing resource-budget benchmarks should be re-run to demonstrate the improvement.

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
| `DecayRateDenominator` | Instance | N/A | N/A | Fixed-point exponential decay denominator λ. Defaults to 1. | [`DEFAULT_DECAY_LAMBDA_DEN`](file:///c:/Users/HP/Desktop/opensource/Ledgerlens-contract/contracts/ledgerlens-score/src/constants.rs#L113) |
