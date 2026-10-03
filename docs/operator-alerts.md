# Operator Alert Rules for LedgerLens Contract Events

This document provides concrete alert thresholds and severity levels for production operators monitoring the LedgerLens smart contract. All rules are derived from contract events and enable proactive detection of anomalies.

## Alert Architecture

Alerts are organized by event category:
- **Operational/Governance**: Contract pause, admin changes, governance actions
- **Data Quality**: Score submission failures, validation issues
- **Configuration**: Parameter changes, model version updates
- **Security**: Signer rotation, authorization failures
- **Performance**: Rate limits, staleness, backlog

Each alert includes:
- **Event Topic**: The contract event that triggers the alert
- **Severity**: Critical, Warning, or Info
- **Threshold**: The condition that triggers the alert
- **Example**: Concrete scenario showing the alert in action
- **Response**: Recommended operator action

## Indexer Health Alerts

The reference event indexer (see `tools/indexer`) ingests contract events from Soroban RPC into Postgres. It exposes three health metrics that operators should alert on: **ledger lag**, **last processed ledger**, and **gap count**. These are emitted as gauges/counters (e.g. Prometheus) and are also queryable from the `indexer_health` table.

### Alert: Indexer Ledger Lag High
- **Metric**: `indexer_ledger_lag` (latest network ledger − last processed ledger)
- **Severity**: Warning
- **Threshold**: `indexer_ledger_lag > 50` for 5 minutes
- **Example**: Indexer stalled after an RPC error; lag climbs to 120 ledgers
- **Response**:
  - Check indexer process health and RPC connectivity
  - Inspect logs for repeated ingestion errors
  - Confirm the cursor in `indexer_cursor` matches the last processed ledger

### Alert: Indexer Ledger Lag Critical
- **Metric**: `indexer_ledger_lag`
- **Severity**: Critical
- **Threshold**: `indexer_ledger_lag > 500` for 5 minutes
- **Example**: Indexer down for over an hour; lag exceeds the RPC retention window
- **Response**:
  - Restart the indexer; it will resume from the persisted cursor
  - If the cursor predates the RPC retention window, trigger backfill from the archive source
  - Verify gap detection fires and backfill completes before lag recovers

### Alert: Indexer Stalled
- **Metric**: `indexer_last_processed_ledger`
- **Severity**: Critical
- **Threshold**: `indexer_last_processed_ledger` unchanged for 10 minutes while network is producing ledgers
- **Example**: Ingestion loop deadlocked; last processed ledger frozen at 1_234_567
- **Response**:
  - Restart the indexer (idempotent upserts make restarts safe)
  - Check for RPC pagination cursor errors in logs
  - Confirm the process is not blocked on a DB lock

### Alert: Indexer Gap Detected
- **Metric**: `indexer_gap_count`
- **Severity**: Warning
- **Threshold**: `indexer_gap_count > 0` (any detected missed ledger range)
- **Example**: RPC returned a cursor jump; ledgers 1_234_600–1_234_650 were skipped
- **Response**:
  - Confirm the automatic backfill path ran for the missing range
  - Verify the archive source is reachable and returned the missing ledgers
  - Ensure `indexer_gap_count` returns to 0 after backfill completes

### Alert: Backfill Failed
- **Metric**: `indexer_backfill_failures_total`
- **Severity**: Critical
- **Threshold**: Any increment in `indexer_backfill_failures_total`
- **Example**: Archive source unreachable; backfill for a detected gap failed
- **Response**:
  - Check archive source availability and credentials
  - Re-run backfill for the recorded gap range once the source recovers
  - Do not advance the cursor past an unbackfilled gap

## Pause Events

### Alert: Contract Pause Detected
- **Event**: `paused`
- **Severity**: Critical
- **Threshold**: Any `paused` event emission
- **Example**: An admin calls `pause()` during suspected security incident
- **Response**: 
  - Verify pause reason with admin team immediately
  - Monitor batch submission failures (will reject with `ContractPaused` code)
  - Check for associated security events or anomalies

### Alert: Contract Unpaused
- **Event**: `unpaused`
- **Severity**: Warning
- **Threshold**: Any `unpaused` event emission after pause duration > 1 hour
- **Example**: Contract was paused for 4 hours; unpaused event detected
- **Response**:
  - Verify unpaused action with admin team
  - Monitor score submissions for any resumption backlog
  - Validate all system dependencies are ready

### Alert: Pair Paused
- **Event**: `pr_pause`
- **Severity**: Warning
- **Threshold**: `paused=true` for critical trading pairs (BTC/USD, ETH/USD, etc.)
- **Example**: XLM/USD pair paused; other pairs still active
- **Response**:
  - Identify the asset pair affected
  - Verify pause duration and expected resolution time
  - Notify downstream consumers (gate callers)

## Signer Churn

### Alert: Signer Added
- **Event**: `sig_add`
- **Severity**: Info
- **Threshold**: New signer registration
- **Example**: New oracle signer registered with public key rotation
- **Response**:
  - Log new signer for audit trail
  - Verify signer meets quorum/threshold requirements
  - Monitor first submissions from new signer

### Alert: Signer Removed
- **Event**: `sig_rem`
- **Severity**: Warning
- **Threshold**: Signer removal when active signers < required threshold + 1
- **Example**: Removing signer reduces active count to exactly the minimum
- **Response**:
  - Verify removal reason with admin
  - Check if replacement signer is being added
  - Alert if quorum would fall below minimum

### Alert: Signer Expiring Soon
- **Event**: `sig_exp`
- **Severity**: Warning
- **Threshold**: Signer TTL approaching expiry (< 24 hours remaining)
- **Example**: Signer key expires in 18 hours; no rotation in progress
- **Response**:
  - Immediately initiate signer rotation
  - Coordinate with signer infrastructure team
  - Monitor for grace period exhaustion

### Alert: Signer Expired
- **Event**: `sig_expd`
- **Severity**: Critical
- **Threshold**: Any `sig_expd` event (signer no longer accepted)
- **Example**: Signer key expired 2 hours ago; submissions now rejected
- **Response**:
  - Emergency: activate backup signer immediately
  - Investigate why rotation wasn't completed
  - Monitor rate of failed submissions due to invalid signer

## Rejection Spikes

### Alert: Batch Rejection Rate Spike
- **Event**: `bat_summ` (batch_processing_summary)
- **Severity**: Warning
- **Threshold**: `rejected_count > accepted_count * 0.2` (>20% rejection rate) in single batch
- **Example**: Batch of 10 entries: 8 accepted, 2 rejected
- **Response**:
  - Analyze rejection codes (`rejected_data`, `rejected_ratelimit`, etc.)
  - Check if external signers are sending malformed data
  - Verify rate limits haven't changed unexpectedly

### Alert: Contract Pause Rejections
- **Event**: `bat_rej_pa` (batch_rejected_contract_paused)
- **Severity**: Critical (if contract should not be paused)
- **Threshold**: Any `bat_rej_pa` events during expected operating hours
- **Example**: Batch submissions failing with pause code at 14:00 UTC
- **Response**:
  - Verify contract pause status immediately
  - Check if pause was intentional (security incident, maintenance)
  - Resume contract if pause was accidental

### Alert: Data Quality Rejections
- **Event**: `bat_rej_dq` (batch_rejected_data_quality)
- **Severity**: Warning
- **Threshold**: `count > 5` rejections per batch due to data quality
- **Example**: 6 entries rejected due to invalid_score (reason_code=1)
- **Response**:
  - Contact signer infrastructure team
  - Verify score calculation/validation pipeline
  - Check if model version changed unexpectedly

### Alert: Model Version Rejections
- **Event**: `bat_rej_mv` (batch_rejected_model_version)
- **Severity**: Warning
- **Threshold**: Any rejections due to `ModelVersionNotRegistered` or `ModelVersionDeprecated`
- **Example**: Batch entries using model v3, but only v4 is active
- **Response**:
  - Verify model version update was deployed to all signers
  - Check if model v3 deprecation was announced to signers
  - Coordinate model upgrade timeline if necessary

### Alert: Rate Limit Rejections
- **Event**: `bat_rej_rl` (batch_rejected_rate_limit)
- **Severity**: Warning
- **Threshold**: `count > 2` rate limit rejections in 5-minute window
- **Example**: Same wallet submitting 3 scores in 10 seconds
- **Response**:
  - Review rate limit configuration
  - Check if legitimate high-frequency signer
  - Consider adjusting limits or granting override

### Alert: Attestation Rejections
- **Event**: `bat_rej_at` (batch_rejected_attestation)
- **Severity**: Critical
- **Threshold**: Any `bat_rej_at` events (invalid attestation)
- **Example**: Batch with invalid merkle proof or bad signature
- **Response**:
  - Verify signer public key is current
  - Check if batch signing infrastructure has issues
  - Inspect batch processing pipeline

## Stale Submissions

### Alert: Oracle Staleness Detected
- **Event**: `orc_stale` (oracle_stale_fallback)
- **Severity**: Warning
- **Threshold**: Oracle unchanged for > staleness_threshold (e.g., 1 hour)
- **Example**: External oracle data hasn't updated in 75 minutes; confidence reduced
- **Response**:
  - Contact external oracle infrastructure team
  - Verify oracle data feed is active
  - Check network connectivity to oracle
  - Monitor gate callers for reduced confidence acceptance

### Alert: Service Silence Alert
- **Event**: `svc_sil` (service_silence_alert)
- **Severity**: Warning
- **Threshold**: Service heartbeat not detected for > threshold (e.g., 30 minutes)
- **Example**: Service last active at 14:20 UTC; alert triggered at 14:52 UTC
- **Response**:
  - Check if service is running
  - Verify network connectivity
  - Restart service if needed
  - Investigate why heartbeat was missed

### Alert: Service Resumed
- **Event**: `svc_res` (service_resumed)
- **Severity**: Info
- **Threshold**: Service recovered after silence event
- **Example**: Service returns online after 18-minute gap
- **Response**:
  - Log gap duration for metrics
  - Verify no critical events were missed during silence
  - Monitor for stability in following minutes

## Upgrade Windows

### Alert: Upgrade Proposed
- **Event**: `upg_prop` (upgrade_proposed)
- **Severity**: Info
- **Threshold**: New upgrade proposal with executable_after timestamp
- **Example**: Upgrade v2.1.0 proposed; executable at 2026-08-01 14:00:00 UTC
- **Response**:
  - Log upgrade details for audit trail
  - Calculate upgrade window (now + executable_after delay)
  - Notify team for change management
  - Monitor for competing proposals (only one pending at a time)

### Alert: Upgrade Executed
- **Event**: `upg_exec` (upgrade_executed)
- **Severity**: Warning
- **Threshold**: Any `upg_exec` event
- **Example**: Upgrade v2.1.0 executed at 2026-08-01 14:05:00 UTC
- **Response**:
  - Verify new contract version is active
  - Monitor for post-upgrade anomalies
  - Check that indexer cursor advanced past the upgrade ledger
  - Validate all dependent services are compatible

## Configuration Changes

### Alert: Parameter Updated
- **Event**: `cfg_upd` (config_updated)
- **Severity**: Info
- **Threshold**: Any configuration parameter change
- **Example**: `staleness_threshold` changed from 3600 to 1800 seconds
- **Response**:
  - Log parameter change for audit trail
  - Verify change was intentional and approved
  - Monitor for behavioral changes in dependent systems

### Alert: Model Version Registered
- **Event**: `mdl_reg` (model_version_registered)
- **Severity**: Info
- **Threshold**: New model version registration
- **Example**: Model v4 registered with new scoring algorithm
- **Response**:
  - Log model version for audit trail
  - Notify signer infrastructure team
  - Coordinate signer upgrade timeline

### Alert: Model Version Deprecated
- **Event**: `mdl_dep` (model_version_deprecated)
- **Severity**: Warning
- **Threshold**: Any model version deprecation
- **Example**: Model v3 deprecated; signers must upgrade to v4
- **Response**:
  - Notify all signers using deprecated version
  - Monitor for rejections due to deprecated model
  - Set deadline for signer upgrades

## Security Events

### Alert: Admin Changed
- **Event**: `adm_chg` (admin_changed)
- **Severity**: Critical
- **Threshold**: Any admin address change
- **Example**: Admin address changed from GABC... to GXYZ...
- **Response**:
  - Verify change with governance team immediately
  - Check for unauthorized access
  - Review all recent admin actions

### Alert: Authorization Failure
- **Event**: `auth_fail` (authorization_failure)
- **Severity**: Warning
- **Threshold**: `count > 3` authorization failures in 5-minute window
- **Example**: 5 failed attempts to call admin function
- **Response**:
  - Investigate source of failed attempts
  - Check if legitimate signer has configuration issue
  - Consider rate limiting or blocking suspicious addresses

## Performance Events

### Alert: Rate Limit Hit
- **Event**: `rl_hit` (rate_limit_hit)
- **Severity**: Info
- **Threshold**: `count > 10` rate limit hits in 5-minute window
- **Example**: 15 rate limit hits from multiple signers
- **Response**:
  - Review rate limit configuration
  - Check if legitimate high-frequency signers
  - Consider adjusting limits if needed

### Alert: Batch Backlog
- **Event**: `bat_backlog` (batch_backlog_detected)
- **Severity**: Warning
- **Threshold**: `backlog_size > 100` pending batches
- **Example**: 150 batches pending processing
- **Response**:
  - Check batch processing infrastructure
  - Verify signer availability
  - Monitor for processing delays

## Alert Severity Summary

| Severity | Response Time | Escalation |
|----------|---------------|------------|
| Critical | Immediate | Page on-call engineer |
| Warning | < 15 minutes | Notify team channel |
| Info | < 1 hour | Log for audit |

## Integration with Monitoring

All alerts should be integrated with your monitoring system (Prometheus, Grafana, PagerDuty, etc.). Event topics map to metrics as follows:

- Contract events → Prometheus counters/gauges
- Indexer health metrics (`indexer_ledger_lag`, `indexer_last_processed_ledger`, `indexer_gap_count`, `indexer_backfill_failures_total`) → Prometheus gauges/counters
- Alert thresholds → Prometheus alert rules
- Escalation → PagerDuty/Opsgenie integration

See `docs/slo-operational-targets.md` for SLO targets and `tools/indexer` for the reference indexer implementation.
