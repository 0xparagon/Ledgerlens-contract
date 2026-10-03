# Key-Rotation Operator Runbook — LedgerLens Score Contract

> **Version:** 1.1 (issues #633, #1180)  
> **Audience:** On-call operators and contract admins  
> **Related:** `scripts/rotate-keys-rehearsal.sh`, `docs/incident-response-runbook.md`, `docs/threshold-attestation-spec.md`

---

## 0. Threshold Attestation Key Ceremony (FROST)

This section covers the distributed key generation (DKG) ceremony used to produce the
threshold attestation aggregate public key. The ceremony is implemented by the
`frost-ceremony` CLI (see `docs/threshold-attestation-spec.md` for the construction
mapping and curve/encoding constraints). No single participant ever holds the full
private key; the contract only ever sees the aggregate public key and a public
transcript.

### 0.1 Participant Selection

| Role | Count | Responsibility |
|------|-------|----------------|
| **Ceremony coordinator** | 1 | Runs the CLI, relays round messages, publishes the transcript. Never a share holder. |
| **Participants (share holders)** | N (e.g. 5) | Each runs the CLI locally, holds exactly one key share. |
| **Witnesses** | ≥ 2 | Independent observers that verify the transcript hash and sign the attestation report. Hold no shares. |

Selection rules:

- Participants must be distinct individuals on distinct administrative domains.
- Threshold `t` must satisfy `t > N/2` and `t ≤ N`; recommended `t = ceil(2N/3)`.
- At least one witness must be from a different team than the coordinator.
- Record each participant's identity, machine fingerprint, and CLI version in the report.

### 0.2 Network Isolation

- Run the ceremony on an isolated network segment (no inbound internet).
- Round messages are exchanged out-of-band (signed files on removable media or a
  dedicated point-to-point channel); the CLI never opens a listening socket.
- Each participant verifies the transcript hash of the previous round before
  contributing to the next round.
- Disable screen sharing, remote shells, and clipboard sync for the duration.

### 0.3 Ceremony Procedure

```bash
# 0. Coordinator: initialise the ceremony and publish the parameters
frost-ceremony init \
  --participants 5 \
  --threshold 4 \
  --curve ed25519 \
  --out ceremony/

# 1. Each participant: round 1 — commit to a random polynomial
frost-ceremony round1 \
  --participant <ID> \
  --ceremony ceremony/ \
  --out ceremony/round1-<ID>.json

# 2. Coordinator: collect round-1 commitments, verify each, then distribute
frost-ceremony verify-round1 --ceremony ceremony/ --round1 ceremony/round1-*.json

# 3. Each participant: round 2 — send encrypted shares to every other participant
frost-ceremony round2 \
  --participant <ID> \
  --ceremony ceremony/ \
  --out ceremony/round2-<ID>.json

# 4. Each participant: verify received shares against round-1 commitments
frost-ceremony verify-shares \
  --participant <ID> \
  --ceremony ceremony/ \
  --shares ceremony/round2-*.json

# 5. Coordinator: finalise, emit aggregate pubkey + public transcript + proof
frost-ceremony finalize \
  --ceremony ceremony/ \
  --out ceremony/transcript.json \
  --proof ceremony/proof-of-correct-generation.json
```

### 0.4 Transcript Hashing & Participant Verification

- Every round message is hashed (SHA-256 over the canonical JSON encoding) and the
  hash is included in the next round's message, forming a hash chain.
- `verify-round1` and `verify-shares` abort the ceremony on any mismatch, invalid
  share, or missing participant.
- The final `transcript.json` contains: ceremony parameters, all round messages,
  the hash chain, the aggregate public key, and each participant's verification
  signature over the transcript hash.
- The `proof-of-correct-generation.json` is the artifact referenced by the
  contract-side registration flow (`rotate_service_pubkey` / threshold registration).

### 0.5 Abort Handling

| Condition | Action |
|-----------|--------|
| Invalid share from a participant | Abort; exclude the participant; restart with a fresh ceremony ID. |
| Participant dropout before round 2 | Abort; restart with `N-1` participants and recomputed `t`. |
| Transcript hash mismatch | Abort; treat as tampering; investigate before restarting. |
| Fewer than `t` valid shares at finalise | Abort; no aggregate key is produced. |

A ceremony that aborts MUST NOT be resumed — always start a new ceremony with a new
ceremony ID so that no partial secret material is reused.

### 0.6 Registration & Rotation

1. Publish `transcript.json` and `proof-of-correct-generation.json` to the ceremony
   archive (immutable storage).
2. Register the aggregate public key via the contract-side registration flow
   (see §3 for `rotate_service_pubkey`).
3. For rotation, run a fresh ceremony (§0.3) and register the new aggregate key with
   an overlap window (§3) so in-flight attestations complete.

### 0.7 Ceremony Checklist

- [ ] Participants, witnesses, and coordinator identified and recorded.
- [ ] Network isolation verified; out-of-band channel tested.
- [ ] CLI version pinned and hashes recorded for all participants.
- [ ] `init` parameters reviewed (`N`, `t`, curve, encoding).
- [ ] Round 1 commitments verified by every participant.
- [ ] Round 2 shares verified against commitments; no invalid shares.
- [ ] Transcript hash chain verified end-to-end.
- [ ] `transcript.json` and `proof-of-correct-generation.json` archived.
- [ ] Aggregate public key registered on-chain; registration tx hash recorded.
- [ ] Attestation report (§0.8) signed by all witnesses.

### 0.8 Attestation Report Template

```markdown
# Threshold Attestation Ceremony Report

- Ceremony ID: <uuid>
- Date (UTC): <YYYY-MM-DDThh:mm:ssZ>
- CLI version: <version> (sha256: <hash>)
- Curve / encoding: ed25519 / <encoding>
- Participants (N): <N>
- Threshold (t): <t>

## Participants
| ID | Name | Domain | Machine fingerprint | Round1 hash | Round2 hash |
|----|------|--------|---------------------|-------------|-------------|
| P1 |      |        |                     |             |             |

## Witnesses
| Name | Team | Transcript hash verified | Signature |
|------|------|--------------------------|-----------|
|      |      |                          |           |

## Artifacts
- Aggregate public key: <hex>
- Transcript: <path / sha256>
- Proof of correct generation: <path / sha256>
- Registration tx: <tx hash>

## Deviations / Aborts
- <none, or describe>

## Sign-off
- Coordinator: <name, signature>
- Witness 1: <name, signature>
- Witness 2: <name, signature>
```

### 0.9 Recovery

- **Lost share:** the participant is excluded; run a fresh ceremony with `N-1`
  participants and recomputed `t`. Never attempt to reconstruct a lost share from
  others outside a ceremony.
- **Compromised participant:** treat the aggregate key as compromised; run a fresh
  ceremony and rotate the aggregate public key (§3) with a short overlap window.
- **Lost transcript:** the ceremony cannot be re-verified; re-run the ceremony. The
  on-chain aggregate key remains valid until rotated.
- **Coordinator failure:** any participant may take over coordination using the
  published round messages; the transcript hash chain guarantees integrity.

---

## 1. Service Signer Rotation

### Trust Assumptions

| Role | Trust Model |
|------|-------------|
| **Admin multisig** (M-of-N) | Authorises all signer additions/removals. Threshold must be > N/2. |
| **Service signers** (M-of-N) | Each service signer may individually authorise score submissions. |
| **New signer onboarding** | The new signer's Stellar keypair must be generated securely off-chain and the public address known before the admin transaction is submitted. |

### Procedure: Add a Service Signer

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  --network $NETWORK \
  -- \
  add_service_signer \
  --admin_signants '["<ADMIN_1>", "<ADMIN_2>"]' \
  --signer "<NEW_SIGNER_ADDRESS>"
```yaml

### Procedure: Remove a Service Signer

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  --network $NETWORK \
  -- \
  remove_service_signer \
  --admin_signants '["<ADMIN_1>", "<ADMIN_2>"]' \
  --signer "<SIGNER_TO_REMOVE>"

**Note:** The threshold auto-adjusts downward if it exceeds the new set size.

### Procedure: Full Rotation (replace entire set)

```bash
# 1. Add new signers
for signer in "<NEW_SIGNER_1>" "<NEW_SIGNER_2>" "<NEW_SIGNER_3>"; do
  soroban contract invoke --id $CONTRACT_ID --source $ADMIN_KEY --network $NETWORK -- \
    add_service_signer --admin_signants '["<ADMIN>"]' --signer "$signer"
done

# 2. Update threshold
soroban contract invoke --id $CONTRACT_ID --source $ADMIN_KEY --network $NETWORK -- \
  set_service_threshold --admin_signants '["<ADMIN>"]' --threshold 2

# 3. Remove old signers
for signer in "<OLD_SIGNER_1>" "<OLD_SIGNER_2>"; do
  soroban contract invoke --id $CONTRACT_ID --source $ADMIN_KEY --network $NETWORK -- \
    remove_service_signer --admin_signants '["<ADMIN>"]' --signer "$signer"
done
```yaml

### Constraints

- Maximum service signers: `MAX_SERVICE_SIGNERS` (10)
- Threshold can never exceed the current set size
- Removing a signer when threshold > new size auto-adjusts threshold

---

## 2. Admin Signer Rotation

### Procedure: Add an Admin Signer

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  --network $NETWORK \
  -- \
  add_admin_signer \
  --admin_signants '["<ADMIN_1>", "<ADMIN_2>"]' \
  --signer "<NEW_ADMIN_SIGNER>"

### Procedure: Remove an Admin Signer

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  --network $NETWORK \
  -- \
  remove_admin_signer \
  --admin_signants '["<ADMIN_1>", "<ADMIN_2>"]' \
  --signer "<ADMIN_TO_REMOVE>"
```yaml

### Constraints

- Maximum admin signers: `MAX_ADMIN_SIGNERS` (5)
- Setting threshold to 0 is not allowed (use remove to fully transition)
- Threshold auto-adjusts when signer removal would leave threshold > set size

---

## 3. Service Pubkey Rotation

### Procedure: Gradual Rotation (with Overlap Window)

```bash
# Rotate with a 24-hour overlap window so in-flight attestations complete
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  --network $NETWORK \
  -- \
  rotate_service_pubkey \
  --admin_signants '["<ADMIN>"]' \
  --new_key "<NEW_65BYTE_PUBKEY>" \
  --overlap_secs 86400

During the overlap window both old and new pubkeys are accepted for attestation verification. After the overlap expires, only the new key is accepted.

### Procedure: Instant Rotation

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  --network $NETWORK \
  -- \
  rotate_service_pubkey \
  --admin_signants '["<ADMIN>"]' \
  --new_key "<NEW_PUBKEY>" \
  --overlap_secs 0
```yaml

### Verification

```bash
# Check if a pending rotation exists
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  --network $NETWORK \
  -- \
  get_pending_service_pubkey

# Get active pubkey
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  --network $NETWORK \
  -- \
  get_service_pubkey

---

## 4. Failure Scenarios & Recovery

### Scenario 1: Signer Loss (Compromised Key)

**Detection:** Monitoring alert from abnormal score submission pattern or security audit.  
**Containment:**

```bash
# 1. Freeze the contract (if available — requires contract version >= 5)
# 2. Remove compromised signer via admin multisig
soroban contract invoke \
  --id $CONTRACT_ID --source $ADMIN_KEY --network $NETWORK -- \
  remove_service_signer \
  --admin_signants '["<ADMIN_1>", "<ADMIN_2>"]' \
  --signer "<COMPROMISED_SIGNER>"

# 3. If threshold was N-of-M and we lost signers, add replacement
soroban contract invoke \
  --id $CONTRACT_ID --source $ADMIN_KEY --network $NETWORK -- \
  add_service_signer \
  --admin_signants '["<ADMIN_1>", "<ADMIN_2>"]' \
  --signer "<NEW_SIGNER>"

# 4. Rotate service pubkey if signer had attestation key access
soroban contract invoke \
  --id $CONTRACT_ID --source $ADMIN_KEY --network $NETWORK -- \
  rotate_service_pubkey \
  --admin_signants '["<ADMIN_1>", "<ADMIN_2>"]' \
  --new_key "<NEW_PUBKEY>" \
  --overlap_secs 3600

# 5. Unfreeze (if frozen)
```yaml

### Scenario 2: Threshold Lost (Too Few Signers)

**Detection:** `submit_score` returns `InsufficientSigners` (14).  
**Recovery:**

```bash
# Add enough signers to meet the threshold
soroban contract invoke \
  --id $CONTRACT_ID --source $ADMIN_KEY --network $NETWORK -- \
  add_service_signer \
  --admin_signants '["<ADMIN>"]' \
  --signer "<NEW_SIGNER>"

### Scenario 3: Rotation Interrupted (Network Failure)

**Detection:** Transaction submission times out or returns unknown error mid-rotation.  
**Recovery:**

```bash
# 1. Verify current state
soroban contract invoke \
  --id $CONTRACT_ID --source $ADMIN_KEY --network $NETWORK -- \
  get_service_signer_count

# 2. Check if signer was partially added
soroban contract invoke \
  --id $CONTRACT_ID --source $ADMIN_KEY --network $NETWORK -- \
  get_service_signers

# 3. Depending on state, either add remaining signers or remove partial ones
```yaml

### Scenario 4: Pubkey Rotation with Stale Signatures

**Detection:** Score submissions fail with `InvalidAttestation` after pubkey rotation.  
**Recovery:** Ensure overlap window is sufficiently long for in-flight submissions. If signatures are already failing:

```bash
# Extend overlap by rotating to the same new key with a fresh overlap window
soroban contract invoke \
  --id $CONTRACT_ID --source $ADMIN_KEY --network $NETWORK -- \
  rotate_service_pubkey \
  --admin_signants '["<ADMIN>"]' \
  --new_key "<CURRENT_NEW_KEY>" \
  --overlap_secs 3600

---

## 5. Rehearsal Automation

Run the key-rotation rehearsal on testnet before any production change:

```bash
./scripts/rotate-keys-rehearsal.sh
```yaml

For a dry run:
```bash
./scripts/rotate-keys-rehearsal.sh --dry-run

To keep the deployment for manual inspection:
```bash
./scripts/rotate-keys-rehearsal.sh --keep-deployment
```yaml

### What the Rehearsal Validates

1. **Service signer rotation** — add signers, set threshold, remove signers, verify auto-adjustment
2. **Admin signer rotation** — add signers, set threshold, remove signers
3. **Signer loss simulation** — remove signer and verify threshold auto-adjusts
4. **Partial failure handling** — invalid signer rejection, threshold > set size rejection
5. **Rollback** — re-add removed signers, restore thresholds
6. **Service pubkey rotation** — set initial key, rotate with overlap, instant rotation
7. **Stale data recovery** — submit score, refresh signer set, verify data persists
8. **Post-action report** — records every action with stable identifiers

---

## 6. Post-Action Report

After every key-rotation operation (production or rehearsal), a post-action report should be generated containing:

- Action log with stable action IDs (T0001, T0002, etc.)
- Pre-rotation signer set and thresholds
- Post-rotation signer set and thresholds
- For threshold attestation ceremonies: the ceremony ID, transcript hash, aggregate
  public key, and the signed attestation report (§0.8)
- Any deviations, aborts, or manual interventions
