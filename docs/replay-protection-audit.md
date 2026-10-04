# Replay-Protection Audit Report

## Executive Summary

An audit of the two independent commit-reveal mechanisms in `contracts/ledgerlens-score` was conducted to evaluate replay attack vulnerability, scope binding, and storage isolation.

**Audit Outcome:** **No gap found.**
All tested replay attack vectors (replaying in later reveal windows, replaying across asset pairs, replaying twice, and cross-mechanism replay) are correctly rejected by existing storage scoping keys, key deletion post-reveal, and storage namespace isolation.

---

## 1. Commitment Scoping Trace

### Mechanism 1: Multi-Model Consensus Commit-Reveal (`commit_consensus` / `reveal_consensus`)

* **Functions:** `commit_consensus` (line 692), `reveal_consensus` (line 712)
* **Storage Key:** `DataKeyC::ConsensusCommitment(model: Address, wallet: Address, asset_pair: Symbol)` in Soroban `temporary()` storage.
* **Storage Scoping:** Keyed explicitly by **`model` (signer) + `wallet` + `asset_pair`**.
* **Storage Lifetime:** Temporary TTL bounded by `get_reveal_window_secs(env)`.
* **On-Chain Key Deletion:** `reveal_consensus` explicitly removes the storage key (`remove_consensus_commitment`) immediately upon verifying a valid commitment.
* **Commitment Hash Byte Construction:**
  * `computed_hash = sha256(score.to_be_bytes() || nonce.to_be_bytes())` (12 bytes buffer: 4 bytes `u32` score + 8 bytes `u64` nonce).
* **Scoping Attributes Summary:**
  * **Wallet:** Included in storage key (`DataKeyC::ConsensusCommitment`).
  * **Asset Pair:** Included in storage key (`DataKeyC::ConsensusCommitment`).
  * **Signer / Model:** Included in storage key (`DataKeyC::ConsensusCommitment`).
  * **Nonce:** Included in `sha256` commitment payload.
  * **Window / Epoch:** Enforced by temporary storage TTL eviction and `remove_consensus_commitment` on consumption.

---

### Mechanism 2: Finality Buffer Commit-Reveal (`auto_commit_score` / `commit_pending_score` / `cancel_pending_score`)

* **Functions:** `submit_score` (holding pending entry), `auto_commit_score` / `commit_pending_score` (line 560), `cancel_pending_score` (line 629).
* **Storage Key:** `DataKeyB::PendingScore(wallet: Address, asset_pair: Symbol)` in Soroban `persistent()` storage.
* **Storage Scoping:** Keyed explicitly by **`wallet` + `asset_pair`**.
* **Data Structure Fields:** `PendingScoreEntry` containing `score`, `benford_flag`, `ml_flag`, `submitted_at`, `confidence`, `model_version`, `timestamp`, `commit_after`, `submitted_by` (signer Address), and `commitment` (`Option<BytesN<32>>`).
* **Promote & Clear:** `commit_pending_score` moves the pending score to live storage (`DataKey::Score`) and immediately calls `clear_pending_score(&env, &wallet, &asset_pair)`, deleting the pending key.
* **Admin Cancellation:** `cancel_pending_score` requires Admin quorum authorization and deletes `DataKeyB::PendingScore`.
* **Scoping Attributes Summary:**
  * **Wallet:** Included in storage key (`DataKeyB::PendingScore`).
  * **Asset Pair:** Included in storage key (`DataKeyB::PendingScore`).
  * **Signer:** Recorded inside `PendingScoreEntry.submitted_by`.
  * **Nonce:** N/A (time-gated administrative hold window).
  * **Window / Epoch:** Enforced by `commit_after` timestamp requirement (`now >= commit_after`) and immediate key deletion upon commit or cancellation.

---

## 2. Storage Key & Namespace Isolation Analysis

* **Consensus Commitments** are stored in `DataKeyC::ConsensusCommitment(model: Address, wallet: Address, asset_pair: Symbol)` under Soroban's **temporary** storage space.
* **Pending Scores** are stored in `DataKeyB::PendingScore(wallet: Address, asset_pair: Symbol)` under Soroban's **persistent** storage space.
* **Namespace Protection:** Soroban contract storage separates keys by enum variant types (`DataKeyC` vs `DataKeyB`) and storage sub-trees (`temporary()` vs `persistent()`).
* **Design-Level Guarantee:** No genuine cross-mechanism replay vector exists to test against at runtime because the two mechanisms use structurally distinct storage keys and storage subtrees by construction. A commitment or pending entry in one path can never be read, misinterpreted, or consumed by the other. This structural isolation is a compile-time design guarantee rather than something demonstrated by runtime replay attempts.

---

## 3. Replay Test Scenarios & Results

All test scenarios were implemented in [`contracts/ledgerlens-score/src/test_replay_audit.rs`](file:///home/mxr/Documents/Ledgerlens-contract/contracts/ledgerlens-score/src/test_replay_audit.rs). 

Tests **1a, 1b, 2a, and 3b** represent legitimate adversarial replay attempts. Tests **2b and 3a** function as runtime storage isolation correctness checks demonstrating that independent storage entries operate concurrently without state interference.

| # | Scenario | Classification | Setup / State | Replay / Operation Attempt | Result | Mechanism Behavior / Error Returned |
|---|----------|----------------|---------------|----------------------------|--------|------------------------------------|
| 1a | Replaying `reveal_consensus` twice | Adversarial Replay | Active commitment | Call `reveal_consensus` a second time after initial successful reveal | **Replay Rejected** | `Error::RevealWindowExpired` (Key deleted on first reveal) |
| 1b | Replaying old commitment/nonce in new window | Adversarial Replay | Active commitment for new score (80) | Attempt reveal using old score (70) & old nonce | **Replay Rejected** | `Error::CommitmentMismatch` |
| 1c | Replaying `commit_pending_score` twice | Adversarial Replay | Active pending score | Call `commit_pending_score` a second time | **Replay Rejected** | `Error::NoPendingScore` (Pending entry cleared on first commit) |
| 2a | Cross-asset pair consensus reveal | Adversarial Replay | Active commitments for both `pair_a` (`hash_a`) and `pair_b` (`hash_b`) | Attempt to satisfy `pair_b` reveal using `pair_a` score, nonce, and hash | **Replay Rejected** | `Error::CommitmentMismatch` (`hash_a != hash_b`) |
| 2b | Cross-asset pair pending score commit | Storage Isolation Check | Active pending scores for both `pair_a` (score 50) and `pair_b` (score 90) | Execute `commit_pending_score(wallet, pair_b)` | **Isolation Verified** | Commits `pair_b` score (90) only; `pair_a` score (50) is isolated in its own key slot and remains pending |
| 3a | Cross-mechanism: `commit_consensus` into `commit_pending_score` | Storage Isolation Check | Active `commit_consensus` (score 90) & active pending score (score 50 carrying consensus hash) | Execute `commit_pending_score` to attempt committing consensus score | **Isolation Verified** | Commits pending score (50) to live storage; consensus commitment (90) remains untouched in `DataKeyC` temporary storage |
| 3b | Cross-mechanism: `submit_score` into `reveal_consensus` | Adversarial Replay | Active pending score (`hash_pending`, score 50) & active consensus commitment (`hash_consensus`, score 90) | Call `reveal_consensus` using pending score payload (score 50, `nonce_pending`) | **Replay Rejected** | `Error::CommitmentMismatch` (`hash_pending != hash_consensus`) |

---

## 4. Conclusion

The audit confirms that commitments in both mechanisms are strictly isolated by their storage keys, data types, and lifecycle bounds:
- **Cross-Pair Replay:** Attempts to satisfy a reveal or commit for `pair_b` using valid inputs from `pair_a` fail because each asset pair keys its own storage slot (`DataKeyC` or `DataKeyB`) and validates hashes against that specific pair's committed hash.
- **Cross-Mechanism Replay & Storage Isolation:** The two mechanisms use structurally distinct storage keys (`DataKeyC` vs `DataKeyB`) and subtrees (`temporary` vs `persistent`), providing a design-level guarantee against cross-mechanism state consumption. Adversarial cross-reveal attempts are rejected with `Error::CommitmentMismatch`.

No security gap was identified.

---

## 5. Update: permissionless attested relay (`relay_attested_score`)

**Date:** 2026-09-29 · **Status:** Implemented — see `rent_relay_credits.rs`,
`test_relay_attestation.rs`.

`submit_score`'s existing single-key `ScoreAttestation` path proves a
submission's cryptographic origin, but the transaction submitter is still
required to independently satisfy `require_auth()` as the service account
(or an M-of-N signer) — so only the service account itself could ever be the
one to actually submit the transaction. This update adds
`relay_attested_score`, a new permissionless entry point any relayer may
call, authorized *solely* by a new, more tightly-bound attestation variant.

### Which variant is relayable

Only the new `RelayScoreAttestation` type is relayable — the legacy
`ScoreAttestation` used by `submit_score` is untouched (no ABI/behavior
change there) precisely because it does not bind every field that affects
state, and loosening its verification to drop `require_auth` would have
been a live security regression rather than an additive feature. See the
finding below.

### Binding (issue requirement: "prove the attestation binds all fields that
### affect state, including nonce, sequence, contract address and network")

`compute_relay_commitment` binds, in the signed SHA-256 preimage: the full
score payload (wallet, asset pair, score, flags, timestamp, confidence,
model version), the contract address, the network passphrase
(`env.ledger().network_id()`), the contract id and version, **and** —
unlike the legacy attestation — `nonce` and `valid_before_ledger` (the
"sequence" freshness bound).

**Finding, documented for transparency:** auditing the legacy
`ScoreAttestation`/`verify_attestation` path (`lib.rs`, `compute_commitment`)
while designing this binding found that `nonce` is checked against stored
state but is *not* part of the signed digest — a signature for a given
score payload is valid for whatever nonce value the caller passes, not
cryptographically tied to one. In practice this is not currently
exploitable on the `submit_score` path *only* because that path still also
requires `require_auth()` from the service account, which independently
prevents anyone else from supplying an arbitrary nonce alongside a replayed
signature. It would become exploitable if that `require_auth` requirement
were ever relaxed without also fixing the binding — which is exactly why
this feature introduces a new attestation type with the fix baked in,
rather than relaxing the legacy path. Tracked here rather than silently
patched, since changing the legacy commitment byte layout is a
compatibility-impacting change to `docs/attestation-spec.md` and existing
off-chain signer integrations, out of scope for this issue.

### Ordering & idempotency

The recomputed commitment digest doubles as an idempotency key
(`RelayedAttestationUsed`, temporary storage). A second relayer posting the
identical attestation gets a single storage read and an early `Ok(false)`
return — no signature verification, no nonce mutation, no tip. `submit_score`
and `relay_attested_score` are otherwise unrelated).

`test_relay_attestation.rs::duplicate_relay_is_a_noop_independent_of_which_relayer_posts_first`
covers this directly: the score state after the first accepted relay is
identical no matter which of two relayer addresses submitted it, and the
second submission is a no-op.

### Threat model coverage

| Threat | Mitigation |
|---|---|
| Griefing (relayer floods duplicate attestations) | Duplicate detection short-circuits before any expensive verification (signature check, nonce read/write, submission pipeline) — the second and later calls are cheap. |
| Censorship by relayers | Permissionless by design — the service can always fall back to `submit_score` directly, or sign for multiple redundant relayers simultaneously; no relayer is privileged. |
| Stale/leaked attestation replayed later | `valid_before_ledger` bounds relayability to a ledger-sequence window; expired attestations are rejected outright (`Error::StaleAttestation`). |
| Key revocation | Rotating the service pubkey (`set_service_pubkey`) takes effect immediately; attestations signed by a revoked key fail signature verification on any subsequent relay attempt. |
| Front-running the score value | The attestation binds the exact score payload; a relayer cannot alter any field without invalidating the signature. |

### Operator guidance: running redundant relayers

Multiple relayers may hold and race the same signed attestation — only the
first to land on-chain does any work; every other submission observes the
same final state at negligible extra cost (one storage read). Operators
wanting redundancy against a single relayer going offline should distribute
the same attestation (and its `valid_before_ledger` bound) to several
independent relayer processes rather than relying on one.

