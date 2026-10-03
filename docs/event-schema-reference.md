# LedgerLens Contract Event Schema Reference

This document provides the complete event schema for all LedgerLens contract events. Use this for implementing event indexers, monitoring systems, and alert pipelines.

## Event Versioning

All events include a `EVENT_VERSION` (currently `1`) in their topic array to enable schema evolution without breaking off-chain systems. Any breaking changes (field reordering, type changes, field removal) will bump the version.

### Versioning Policy for Off-Chain Consumers

The typed decoding crate (`ledgerlens-events`) is generated from the machine-readable event schema in CI, so it cannot drift from the contract. Consumers must follow this policy:

- **Known versions** are decoded into strongly typed Rust structs/enums.
- **Unknown versions** are **not** an error. Decoders return an opaque value (`OpaqueEvent`) that preserves the raw topics and data so callers can log, forward, or handle them without panicking.
- **Adding a new event or bumping `EVENT_VERSION`** requires updating the schema; CI fails if the contract emits an event the schema or crate does not describe.
- **Breaking changes** (field reordering, type changes, field removal) bump `EVENT_VERSION`. Non-breaking additions keep the current version.

### Decoding Example

```rust
use ledgerlens_events::{decode_rpc_event, decode_replay_event, DecodedEvent};

// From a Soroban RPC `getEvents` response entry:
let decoded = decode_rpc_event(&rpc_event)?;
match decoded {
    DecodedEvent::Score(e) => println!("score={} confidence={}", e.score, e.confidence),
    DecodedEvent::Breach(e) => println!("breach on {} score={}", e.asset_pair, e.score),
    // Unknown schema versions are surfaced as opaque, never an error:
    DecodedEvent::Opaque(o) => println!("unknown event version {}: {:?}", o.version, o.topics),
    _ => {}
}

// From replay tool output (same typed surface):
let decoded = decode_replay_event(&replay_record)?;
```

Golden fixtures for every event topic live under `tools/schema-gen/fixtures/`, and a compatibility test decodes each fixture across supported schema versions.

## Event Categories

### 1. Operational/Governance Events

#### `paused`
- **Topic**: `("paused", EVENT_VERSION)`
- **Data**: `Address` (admin who paused)
- **Use Case**: Alert when contract enters paused state
- **Example**: Admin paused contract during security incident

#### `unpaused`
- **Topic**: `("unpaused", EVENT_VERSION)`
- **Data**: `Address` (admin who unpaused)
- **Use Case**: Monitor contract resumption and duration of pause
- **Example**: Contract operational again after 2-hour maintenance

#### `pr_pause`
- **Topic**: `("pr_pause", EVENT_VERSION, asset_pair)`
- **Data**: `bool` (paused status)
- **Use Case**: Track which pairs are operational
- **Example**: Pair "BTC/USD" paused=true

### 2. Score Submission Events

#### `score`
- **Topic**: `("score", EVENT_VERSION, wallet, asset_pair)`
- **Data**: `(score, benford_flag, ml_flag, confidence, timestamp)`
- **Use Case**: Track all score submissions with anomaly flags
- **Example**: Score=42, confidence=85, no anomalies

#### `bat_ok`
- **Topic**: `("bat_ok", merkle_root)`
- **Data**: `(accepted: u32, rejected: u32)`
- **Use Case**: Monitor batch processing success rate
- **Example**: Batch accepted=98 entries, rejected=2

#### `bat_summ`
- **Topic**: `("bat_summ",)`
- **Data**: `(accepted, rejected_pause, rejected_data, rejected_model, rejected_ratelimit, rejected_attestation, rejected_gate)`
- **Use Case**: Aggregate rejection statistics per category
- **Example**: 95 accepted, 2 rejected_data, 1 rejected_ratelimit

#### `bat_rej_pa`
- **Topic**: `("bat_rej_pa",)`
- **Data**: `u32` (count)
- **Use Case**: Alert when contract pause causes rejections
- **Example**: 5 entries rejected due to pause

#### `bat_rej_dq`
- **Topic**: `("bat_rej_dq",)`
- **Data**: `(reason_code: u32, count: u32)`
- **Reason Codes**: 1=invalid_score, 2=invalid_confidence, 3=invalid_timestamp
- **Use Case**: Detect data quality issues from signers
- **Example**: 3 entries rejected for invalid_confidence

#### `bat_rej_mv`
- **Topic**: `("bat_rej_mv",)`
- **Data**: `(reason_code: u32, count: u32)`
- **Reason Codes**: 1=not_registered, 2=deprecated
- **Use Case**: Alert on model version synchronization issues
- **Example**: 2 entries rejected with deprecated model

#### `bat_rej_rl`
- **Topic**: `("bat_rej_rl",)`
- **Data**: `u32` (count)
- **Use Case**: Detect rate limit violations
- **Example**: 1 entry rejected due to rate limit

#### `bat_rej_at`
- **Topic**: `("bat_rej_at",)`
- **Data**: `u32` (count)
- **Use Case**: Alert on signature/attestation failures
- **Example**: 1 entry rejected with invalid attestation

#### `bat_rej_gt`
- **Topic**: `("bat_rej_gt",)`
- **Data**: `u32` (count)
- **Use Case**: Track gate enforcement rejections
- **Example**: 4 entries rejected by gate threshold

### 3. Risk Threshold & Configuration Events

#### `thresh`
- **Topic**: `("thresh", EVENT_VERSION)`
- **Data**: `(old_threshold: u32, new_threshold: u32)`
- **Use Case**: Audit threshold changes
- **Example**: Threshold changed from 75 to 65

#### `breach`
- **Topic**: `("breach", EVENT_VERSION, wallet)`
- **Data**: `(asset_pair, score, threshold)`
- **Use Case**: Monitor threshold breaches
- **Example**: Wallet score=92 exceeded threshold=75

#### `brc_rst`
- **Topic**: `("brc_rst", wallet, asset_pair)`
- **Data**: `Address` (admin who reset)
- **Use Case**: Audit breach counter resets
- **Example**: Admin reset breach counter for XLM/USD pair

#### `cd_upd`
- **Topic**: `("cd_upd", EVENT_VERSION)`
- **Data**: `u64` (cooldown in seconds)
- **Use Case**: Track cooldown period changes
- **Example**: Cooldown changed to 3600 seconds

#### `pcd_upd`
- **Topic**: `("pcd_upd", asset_pair)`
- **Data**: `u64` (pair-specific cooldown)
- **Use Case**: Monitor pair-specific configuration
- **Example**: XLM/USD cooldown=7200

### 4. Signer Management Events

#### `sig_add`
- **Topic**: `("sig_add", EVENT_VERSION)`
- **Data**: `Address` (new signer)
- **Use Case**: Track signer additions
- **Example**: New oracle signer registered

#### `sig_rem`
- **Topic**: `("sig_rem", EVENT_VERSION)`
- **Data**: `Address` (removed signer)
- **Use Case**: Monitor signer removals
- **Example**: Signer retired after rotation

#### `sig_exp`
- **Topic**: `("sig_exp",)`
- **Data**: `Address` (expiring signer)
- **Use Case**: Alert on imminent signer expiration
- **Example**: Signer key expires in 18 hours

#### `sig_expd`
- **Topic**: `("sig_expd",)`
- **Data**: `Address` (expired signer)
- **Use Case**: Alert when signer is no longer valid
- **Example**: Signer no longer accepted; submissions will fail

#### `sig_thr`
- **Topic**: `("sig_thr", EVENT_VERSION)`
- **Data**: `u32` (required number of signers)
- **Use Case**: Track quorum changes
- **Example**: Quorum requirement changed to 7

### 5. Upgrade Events

#### `upg_prop`
- **Topic**: `("upg_prop", EVENT_VERSION)`
- **Data**: `(new_wasm_hash: BytesN<32>, executable_after: u64)`
- **Use Case**: Log upgrade proposals with execution time
- **Example**: Upgrade v2.1.0 proposed; ready at timestamp 1700086400

#### `upg_exec`
- **Topic**: `("upg_exec", EVENT_VERSION)`
- **Data**: `BytesN<32>` (new WASM hash)
- **Use Case**: Confirm successful upgrade execution
- **Example**: Upgrade 0xabcd... executed

#### `upg_veto`
- **Topic**: `("upg_veto", EVENT_VERSION)`
- **Data**: `Address` (admin who vetoed)
- **Use Case**: Track rejected upgrades
- **Example**: Admin vetoed upgrade 2 days early

#### `upg_appr`
- **Topic**: `("upg_appr", signer)`
- **Data**: `(approval_count: u32, required_count: u32)`
- **Use Case**: Monitor multi-sig approval progress
- **Example**: 5 of 7 signatures collected

### 6. Parameter Governance Events

#### `prm_prop`
- **Topic**: `("prm_prop",)`
- **Data**: `(proposal_id: u64, param_key: Symbol, executable_after: u64)`
- **Use Case**: Log parameter change proposals
- **Example**: Proposal ID=42 for "cooldown"; executable in 24h

#### `prm_exec`
- **Topic**: `("prm_exec",)`
- **Data**: `(proposal_id: u64, param_key: Symbol)`
- **Use Case**: Confirm parameter changes applied
- **Example**: Proposal 42 "cooldown" executed

#### `prm_veto`
- **Topic**: `("prm_veto",)`
- **Data**: `(proposal_id: u64, admin: Address)`
- **Use Case**: Track rejected parameter proposals
- **Example**: Proposal 42 vetoed by admin

### 7. Oracle Events

#### `orc_reg`
- **Topic**: `("orc_reg", asset_pair)`
- **Data**: `Address` (oracle contract)
- **Use Case**: Track oracle registrations
- **Example**: Oracle for BTC/USD registered

#### `orc_rem`
- **Topic**: `("orc_rem",)`
- **Data**: `Symbol` (asset_pair)
- **Use Case**: Monitor oracle removals
- **Example**: XLM/USD oracle deregistered

#### `orc_stale`
- **Topic**: `("orc_stale", asset_pair)`
- **Data**: `(last_updated_ts: u64, threshold_secs: u64)`
- **Use Case**: Alert on stale oracle data
- **Example**: Oracle data 65 minutes old; threshold=60 minutes

#### `orc_sthr`
- **Topic**: `("orc_sthr",)`
- **Data**: `u64` (staleness threshold in seconds)
- **Use Case**: Track staleness threshold changes
- **Example**: Staleness threshold changed to 3600 seconds

### 8. Service Heartbeat Events

#### `svc_sil`
- **Topic**: `("svc_sil",)`
- **Data**: `ServiceSilenceAlertEvent { last_active_at, silent_secs, threshold_secs }`
- **Use Case**: Alert when service is not reporting
- **Example**: Service silent for 35 minutes; threshold=30 minutes

#### `svc_res`
- **Topic**: `("svc_res",)`
- **Data**: `ServiceResumedEvent { last_active_at, gap_secs }`
- **Use Case**: Track service recovery and gap duration
