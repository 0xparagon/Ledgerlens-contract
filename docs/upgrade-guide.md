# Operator WASM Upgrade Runbook

## Overview

A WASM upgrade replaces the contract's code on-chain, enabling bug fixes, feature additions, and security patches. Upgrades are high-stakes operations requiring a time-lock delay, admin authorization, and careful coordination with integrators. This guide walks you through the process step-by-step.

**Key principles:**
- **Admin-only:** Only the admin key can propose and execute upgrades.
- **Time-locked:** An upgrade must wait 48 hours (default, configurable) before it can be executed. This window allows integrators to prepare and gives time to cancel if issues are discovered.
- **One-at-a-time:** Only one upgrade proposal can be pending; veto or execute it before proposing a new one.
- **Irreversible:** On-chain WASM upgrades cannot be automatically rolled back. If the new code is broken, you must re-propose the previous WASM.

---

## Expedited Security-Patch Lane

The standard 48-hour timelock is the right default, but it is too slow to respond to a live exploit. The **expedited lane** shortens the delay *only* when a set of compensating controls hold. It is a break-glass path, not a routine deployment channel.

### When to use the lane

Use the expedited lane **only** for an active or imminent security incident where the standard timelock would leave funds or state exposed, for example:

- A live exploit draining funds or corrupting scores.
- A critical vulnerability that is publicly known and trivially exploitable.
- A compromised dependency or admin path that must be neutralized immediately.

Do **not** use the lane for feature releases, gas optimizations, refactors, or any change that can wait 48 hours. Misuse is itself a governance incident (see *Follow-up governance event* below).

### Preconditions

All of the following must hold before an expedited proposal is accepted:

- [ ] The contract is initialized and an admin set exists.
- [ ] No other upgrade proposal is pending (standard or expedited).
- [ ] The expedited cooldown has elapsed since the last expedited execution.
- [ ] The proposal is signed by at least the **expedited quorum** (a strict superset of the standard admin threshold — e.g. a supermajority of admins rather than a simple majority).
- [ ] The new WASM hash is already installed on-chain and matches the proposed hash.

### Quorum and delay floor

- **Quorum:** the expedited lane requires a **higher approval threshold** than the standard lane. Where the standard lane accepts the normal admin threshold, the expedited lane requires the configured expedited quorum (supermajority of admins). Insufficient signatures are rejected with `Error::InsufficientExpeditedQuorum`.
- **Minimum delay floor:** the expedited delay is shorter than the standard delay but may **never** be reduced below the configured floor (`expedited_delay_floor`). The effective delay is `max(expedited_delay, expedited_delay_floor)`. The floor is a hard invariant: no expedited proposal can become executable before `proposed_at + expedited_delay_floor`.
- **Scope:** the expedited lane can only shorten **its own** delay. It cannot modify, shorten, or bypass the standard upgrade delay, the veto window, or any other timelock in the system.

### Cooldown

To prevent the lane from becoming a routine deployment path, expedited executions are rate-limited by a **cooldown**. After an expedited upgrade executes, no new expedited proposal may be accepted until `cooldown` seconds have elapsed. Attempts during the cooldown are rejected with `Error::ExpeditedCooldownActive`. The cooldown applies only to the expedited lane and does not affect standard proposals.

### Effect on the system during the pause window

While an expedited proposal is pending, the contract enters a **mandatory pause window**:

- All **state-changing** entry points (score submission, admin/settings mutation, standard upgrade propose/execute) are **blocked** and return `Error::ContractPaused`.
- **Read-only** entry points (`get_score`, `get_pending_upgrade`, `get_version`, risk-gate queries) remain available so integrators and monitors can observe state.
- The pause is enforced for the entire expedited window, from proposal acceptance until execution or veto.
- The **veto surface is broadened**: any single admin may veto an expedited proposal at any time before execution, and the veto immediately lifts the pause and clears the proposal.

### Post-upgrade automatic verification checklist

Immediately after an expedited execution, the contract runs (and operators must confirm) the following automatic checks before the pause is lifted:

- [ ] New code hash matches the proposed hash.
- [ ] Contract responds to a read-only probe (`get_version` / `get_score`) without trapping.
- [ ] Admin set and critical configuration are intact.
- [ ] No state-changing call reverts unexpectedly once the pause is lifted.
- [ ] Integrator smoke test (risk gate enforced) passes on testnet before mainnet reliance.

If any check fails, treat it as a failed upgrade and re-propose the previous WASM through the standard lane.

### Mandatory follow-up governance event

Every expedited execution **must** be followed by a governance event: a post-incident review published to the governance forum within the incident SLA, covering the trigger, the compensating controls that held, the verification results, and a decision on whether to keep or tighten the lane. The expedited execution emits an `expedited_upgrade_executed` event that references this follow-up obligation.

---

## Pre-Upgrade Checklist

Before proposing an upgrade, complete the following:

### 1. Verify Admin Access
- [ ] Admin key is available and accessible.
- [ ] Admin key has been added to your signing setup (e.g., Ledger, HSM, keystore).
- [ ] You can sign transactions with the admin key (test with a dummy transaction if unsure).

### 2. Verify New WASM Binary
- [ ] New WASM is built: `cargo build --target wasm32-unknown-unknown --release`
- [ ] Binary is located at `target/wasm32-unknown-unknown/release/ledgerlens_score.wasm`
- [ ] WASM hash is computed and noted (see Step 2 below).
- [ ] WASM is tested in a staging/testnet environment first.

### 3. Announce to Integrators
- [ ] Notify all integrators (AMMs, lending protocols, indexers) of the upcoming upgrade at least **48 hours** in advance.
- [ ] Provide a summary of changes: bug fixes, new features, breaking changes.
- [ ] Include the time window: "Upgrade will be executable after [timestamp]."
- [ ] Request acknowledgment of receipt.

### 4. Confirm No In-Flight Transactions
- [ ] Check that there are no critical transactions in-flight that depend on the current contract version.
- [ ] Wait for any ongoing batch score submissions to complete.

### 5. Back Up Contract State (Off-Chain)
- [ ] Document the current contract state:
  - Admin address: `client.get_admin()`
  - Service address: `client.set_service()` (if known)
  - Pending upgrade (if any): `client.get_pending_upgrade()`
  - Upgrade delay: `client.get_upgrade_delay()` = **172,800 seconds** (48 hours)
- [ ] Export a snapshot of critical configuration values.
- [ ] These backups help with diagnostics if the upgrade fails.

---

## Step 1 — Build the New WASM

```bash
cd /path/to/ledgerlens-contract
cargo build --target wasm32-unknown-unknown --release
```yaml

**Output:** `target/wasm32-unknown-unknown/release/ledgerlens_score.wasm`

**Verify the binary exists and is non-empty:**
```bash
ls -lh target/wasm32-unknown-unknown/release/ledgerlens_score.wasm

---

## Step 2 — Compute the WASM Hash

The `propose_upgrade` function requires a 32-byte SHA-256 hash of the new WASM binary. Compute it:

```bash
# Option A: Using soroban contract install (this also uploads the WASM and gives you the hash)
soroban contract install \
  --network testnet \
  --source-account YOUR_ACCOUNT_ADDRESS \
  --wasm target/wasm32-unknown-unknown/release/ledgerlens_score.wasm

# Output will show: "WasmHash: abc123...def789" (64 hex characters)
```yaml

**Option B: Using sha256sum (if you prefer to compute locally without uploading):**
```bash
sha256sum target/wasm32-unknown-unknown/release/ledgerlens_score.wasm
# Output: abc123...def789  ledgerlens_score.wasm

**Note:** If using `soroban contract install`, the WASM is uploaded to the network immediately. You can then propose the upgrade with confidence that the WASM is available.

**Save the hash** (you will use it in Step 3):
```
WASM_HASH=abc123...def789

---

## Step 3 — Propose the Upgrade

The `propose_upgrade` function registers a new WASM to be executed after the delay elapses. This function requires admin authorization.

```bash
soroban contract invoke \
  --network testnet \
  --source-account YOUR_ADMIN_ADDRESS \
  --contract-id LEDGERLENS_CONTRACT_ID \
  -- propose_upgrade \
  --admin-signers '[ADMIN_ADDRESS_1, ADMIN_ADDRESS_2, ...]' \
  --new-wasm-hash WASM_HASH
```yaml

**Parameters:**
- `admin-signers`: List of admin addresses that authorize this proposal. Can be a single admin or a multisig set.
- `new-wasm-hash`: The 32-byte SHA-256 hash computed in Step 2 (as a byte array or hex string).

**Expected result:**
- Function returns `Ok(())` if the proposal is successfully registered.
- Event `upgrade_proposed` is emitted with the proposal details.
- The proposal becomes executable after **172,800 seconds** (48 hours) from the current ledger timestamp.

**If it fails:**
- `Error::NotInitialized`: Contract has no admin yet (initialize first).
- `Error::UpgradeAlreadyPending`: An upgrade proposal is already in flight. Veto it first or wait for it to be executed.
- `Error::Unauthorized`: Admin signature verification failed. Check that all required signers authorized the transaction.

---

## Step 4 — Monitor the Delay

The upgrade cannot be executed until the delay has elapsed. Use this window to:

### Check Remaining Time

```bash
soroban contract invoke \
  --network testnet \
  --source-account YOUR_ACCOUNT_ADDRESS \
  --contract-id LEDGERLENS_CONTRACT_ID \
  -- get_pending_upgrade

**Response** (if a proposal exists):
```json
{
  "new_wasm_hash": "0xabc123...def789",
  "proposed_at": 1234567890,
  "executable_after": 1234567890 + 172800 = 1234740690
}
```yaml

**Calculate remaining time:**
remaining_secs = executable_after - current_ledger_timestamp
remaining_hours = remaining_secs / 3600
```yaml

### If You Need to Cancel

If a critical issue is discovered during the delay window, veto the proposal:

```bash
soroban contract invoke \
  --network testnet \
  --source-account YOUR_ADMIN_ADDRESS \
  --contract-id LEDGERLENS_CONTRACT_ID \
  -- veto_upgrade \
  --admin-signers '[ADMIN_ADDRESS]'

This clears the pending proposal. You can then propose a new WASM if needed.

---

## Step 5 — Execute the Upgrade

Once the delay has elapsed, execute the upgrade:

```bash
soroban contract invoke \
  --network testnet \
  --source-account YOUR_ADMIN_ADDRESS \
  --contract-id LEDGERLENS_CONTRACT_ID \
  -- execute_upgrade \
  --admin-signers '[ADMIN_ADDRESS_1, ADMIN_ADDRESS_2, ...]'
```yaml

**Expected result:**
- The contract's WASM code is replaced with the new binary.
- Event `upgrade_executed` is emitted.
- All state (scores, admins, settings) is preserved.
- New code is active immediately.

**If it fails:**
- `Error::UpgradeNotReady`: The delay has not elapsed yet. Wait longer.
- `Error::NoPendingUpgrade`: There is no proposal to execute. Propose first.
- `Error::Unauthorized`: Admin signature verification failed.

---

## Step 6 — Post-Upgrade Verification

### Verify Contract Still Works

```bash
soroban contract invoke \
  --network testnet \
  --source-account YOUR_ACCOUNT_ADDRESS \
  --contract-id LEDGERLENS_CONTRACT_ID \
  -- get_score \
  --wallet SOME_WALLET \
  --asset-pair XLM_USDC

Expected: The function returns a valid score (or `ScoreNotFound` if the wallet has no score, but the contract is responsive).

### Check Contract Version (if applicable)

```bash
soroban contract invoke \
  --network testnet \
  --source-account YOUR_ACCOUNT_ADDRESS \
  --contract-id LEDGERLENS_CONTRACT_ID \
  -- get_version
```yaml

This returns the contract version number (e.g., `2` or `3`). If your new WASM incremented the version, you should see the new number.

### Verify Integrator Compatibility

- [ ] Contact a few key integrators and confirm their contracts can still call `query_risk_gate`, `get_score`, etc.
- [ ] Run an AMM swap on testnet and verify the risk gate is enforced.
- [ ] Check indexers and off-chain services; confirm they can still parse contract events and data.

---

## Rollback Options

**On-chain WASM upgrades are not au

/* … truncated 2985 chars — edit only what you need near the top … */
