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

## Direct RPC State Reads: Ledger-Key Encoding

Read-heavy consumers and indexers can read contract storage directly through RPC ledger-entry queries (`getLedgerEntries`) instead of invoking the contract. This is cheaper and parallelisable, but requires constructing the exact storage key and decoding the returned value. This section specifies the exact XDR encoding of every key family so that keys can be built and values decoded without reverse-engineering the storage enums.

### LedgerKey structure

A Soroban contract-data ledger key is a `LedgerKey` of type `CONTRACT_DATA`:

```
LedgerKey::ContractData {
    contract: ScAddress,   // ScAddress::Contract(contract_id)
    key:       ScVal,      // the storage key (see below)
    durability: ContractDataDurability, // TEMPORARY | PERSISTENT
}
```

Instance storage is **not** addressed with a `CONTRACT_DATA` key. It is read via `LedgerKey::ContractData` with the special key `ScVal::LedgerKeyContractInstance`, or more commonly via `LedgerKey::ContractInstance { contract }`.

### The multi-enum storage key split

LedgerLens stores keys as a `DataKey` enum. Soroban encodes a Rust enum as an `ScVal::Vec` whose first element is the variant index (`ScVal::U32`) followed by the variant's payload fields in declaration order. The variant index is the **zero-based position** of the variant in the `DataKey` enum in `contracts/ledgerlens-score/src/types.rs`.

For example, a unit variant `DataKey::Paused` at index `8` encodes as:

```
ScVal::Vec([ ScVal::U32(8) ])
```

A tuple variant `DataKey::Score(Address)` at index `12` encodes as:

```
ScVal::Vec([ ScVal::U32(12), ScVal::Address(addr) ])
```

> [!IMPORTANT]
> The variant index is positional. **Adding, removing, or reordering a variant changes the encoding of every subsequent key.** See the compatibility rules below.

### Encoding table

| Key family | Variant index | Payload | Durability | Stability |
| :--- | :---: | :--- | :--- | :--- |
| `Admin` | 0 | — | Instance | Stable |
| `AdminSet` | 1 | — | Instance | Stable |
| `AdminThreshold` | 2 | — | Instance | Stable |
| `Service` | 3 | — | Instance | Stable |
| `ServiceSet` | 4 | — | Instance | Stable |
| `ServiceThreshold` | 5 | — | Instance | Stable |
| `ServicePubKey` | 6 | — | Instance | Stable |
| `SignerTier(Address)` | 7 | `ScVal::Address` | Instance | Stable |
| `Paused` | 8 | — | Instance | Stable |
| `PendingAdmin` | 9 | — | Instance | Stable |
| `RiskThreshold` | 10 | — | Instance | Stable |
| `JumpThreshold` | 11 | — | Instance | Stable |
| `Score(Address)` | 12 | `ScVal::Address` | Persistent | Stable |
| `ScoreHistory(Address)` | 13 | `ScVal::Address` | Persistent | Stable |
| `LastUpdate(Address)` | 14 | `ScVal::Address` | Persistent | Stable |
| `Cooldown(Address)` | 15 | `ScVal::Address` | Temporary | Internal |
| `Embargo(Address)` | 16 | `ScVal::Address` | Persistent | Stable |
| `RiskBand(Address)` | 17 | `ScVal::Address` | Persistent | Stable |
| `HistoryMaxDepth` | 18 | — | Instance | Stable |
| `ContractVersion` | 19 | — | Instance | Stable |
| `PendingUpgrade` | 20 | — | Instance | Stable |
| `UpgradeDelay` | 21 | — | Instance | Stable |
| `StalenessWindow` | 22 | — | Instance | Stable |
| `CooldownSecs` | 23 | — | Instance | Stable |
| `DecayRateNumerator` | 24 | — | Instance | Stable |
| `DecayRateDenominator` | 25 | — | Instance | Stable |

> [!NOTE]
> The indices above are illustrative of the encoding scheme. The authoritative source of truth is the declaration order of `DataKey` in `contracts/ledgerlens-score/src/types.rs`; the fixture vectors in `docs/sdk-conformance-fixtures.md` are validated in CI against that enum on every change.

### Worked hex examples

Given a contract id `C...` (32-byte `ScAddress::Contract`), the following keys encode as shown. The `ScVal` bytes are the XDR of the `key` field; the full `LedgerKey` wraps them with the contract address and durability.

**Unit key — `Paused` (index 8):**

```
ScVal::Vec([ ScVal::U32(8) ])
XDR: 00 00 00 11 00 00 00 01 00 00 00 03 00 00 00 08
     ^vec  ^len=1  ^u32 tag  ^value=8
```

**Address key — `Score(addr)` (index 12):**

```
ScVal::Vec([ ScVal::U32(12), ScVal::Address(addr) ])
XDR: 00 00 00 11 00 00 00 02 00 00 00 03 00 00 00 0c <addr-xdr>
     ^vec  ^len=2  ^u32 tag  ^value=12
```

**Instance key — `Admin` (index 0):**

```
ScVal::Vec([ ScVal::U32(0) ])
XDR: 00 00 00 11 00 00 00 01 00 00 00 03 00 00 00 00
```

### Decoding values

Values are decoded by matching the `ScVal` type against the expected Rust type for the key family:

| Key family | Value `ScVal` | Rust type |
| :--- | :--- | :--- |
| `Admin`, `Service`, `PendingAdmin` | `ScVal::Address` | `Address` |
| `AdminSet`, `ServiceSet` | `ScVal::Vec` of `ScVal::Address` | `Vec<Address>` |
| `AdminThreshold`, `ServiceThreshold` | `ScVal::U32` | `u32` |
| `ServicePubKey` | `ScVal::Bytes` | `BytesN<33>` |
| `SignerTier(Address)` | `ScVal::Map` | `TierBounds` |
| `Paused` | `ScVal::Bool` | `bool` |
| `RiskThreshold`, `JumpThreshold` | `ScVal::U32` | `u32` |
| `Score(Address)` | `ScVal::U32` | `u32` |
| `ScoreHistory(Address)` | `ScVal::Vec` of `ScVal::U32` | `Vec<u32>` |
| `LastUpdate(Address)` | `ScVal::U64` | `u64` |
| `Cooldown(Address)` | `ScVal::U64` | `u64` |
| `Embargo(Address)` | `ScVal::Bool` | `bool` |
| `RiskBand(Address)` | `ScVal::U32` | `u32` |
| `HistoryMaxDepth`, `ContractVersion` | `ScVal::U32` | `u32` |
| `PendingUpgrade` | `ScVal::Map` | `UpgradeProposal` |
| `UpgradeDelay`, `StalenessWindow`, `CooldownSecs` | `ScVal::U64` | `u64` |
| `DecayRateNumerator`, `DecayRateDenominator` | `ScVal::U64` | `u64` |

### TTL and archival state

Every `CONTRACT_DATA` ledger entry returned by `getLedgerEntries` carries a `liveUntilLedgerSeq` field:

- **Live**: `liveUntilLedgerSeq > current_ledger`. The value is present and readable.
- **Archived**: the entry is absent from the response. For `PERSISTENT` durability the entry can be restored with a `RestoreFootprint` operation; for `TEMPORARY` durability the entry is gone permanently and must be rewritten.
- **Instance**: read via `LedgerKey::ContractInstance`; its TTL is the contract instance's `liveUntilLedgerSeq`.

Consumers should treat a missing persistent entry as *archived* (restorable) and a missing temporary entry as *absent* (must be re-created).

### Stability and compatibility rules

Keys are classified as **Stable** or **Internal** in the encoding table above.

- **Stable keys** are part of the public storage ABI. Their variant index, payload shape, durability, and value type will not change without a major version bump and a documented migration. New stable keys are only appended at the end of the enum so existing indices are preserved.
- **Internal keys** (e.g. `Cooldown`) may change encoding between minor versions. Consumers must not depend on their exact layout.

Any change to the `DataKey` enum must update this table, the fixture vectors in `docs/sdk-conformance-fixtures.md`, and the conformance tests, and must follow the repository's compatibility policies.

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
