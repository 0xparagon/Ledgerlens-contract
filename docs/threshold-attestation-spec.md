# Threshold Signature Attestation Specification

Version: 1.0 — introduced in contract ABI version 4.

## 1. Background and motivation

The M-of-N service signing model (`add_service_signer` / `set_service_threshold`) requires each of the M authorised signers to call `require_auth` individually. In Soroban, every `require_auth` call implies a separate Stellar account authorization, so a 3-of-5 setup produces five authorization entries per `submit_score` invocation. This:

- Inflates the transaction size by O(N) signatures.
- Requires all M signers to be online and coordinate for every submission.
- Is observable — on-chain indexers can see exactly which signers participated.

Threshold ECDSA aggregation replaces all of that with a **single 65-byte `(r, s, v)` secp256k1 signature** over the same payload commitment, recoverable to a single *aggregate* public key that the admin pre-registers on-chain.

---

## 2. Cryptographic protocol (off-chain)

Any t-of-n ECDSA threshold scheme that produces a standard secp256k1 `(r, s)` pair may be used. The two most common choices are:

### 2a. FROST (Flexible Round-Optimised Schnorr Threshold)

FROST produces Schnorr signatures, not ECDSA. It is **not** compatible with Stellar's `secp256k1_recover` host function, which only processes ECDSA. Use option 2b instead.

### 2b. GG18 / GG20 threshold ECDSA

GG18 (Gennaro–Goldfeder 2018) and its improved variant GG20 produce standard `(r, s)` ECDSA pairs verifiable with any secp256k1 implementation. The aggregate public key is derived from the key-generation ceremony and is a normal secp256k1 point — indistinguishable from a single-party key to the verifier.

**Recommended library:** [multi-party-ecdsa](https://github.com/ZenGo-X/multi-party-ecdsa) (Rust, MIT).

### 2c. Construction selection and contract mapping

The contract verifies a single secp256k1 ECDSA signature via `secp256k1_recover` (see §3). The ceremony tooling MUST therefore produce a construction whose output maps directly onto that verifier:

| Contract requirement | Required construction property |
|----------------------|-------------------------------|
| `secp256k1_recover(C, r \|\| s, v)` | Standard ECDSA over secp256k1, 32-byte `r`, 32-byte `s`, `v ∈ {0,1}` |
| Aggregate pubkey comparison | Uncompressed 65-byte or compressed 33-byte SEC-1 point |
| Commitment preimage | SHA-256 over the `ScoreAttestation` layout (see `docs/attestation-spec.md`) |

**Selected construction:** GG20 threshold ECDSA (secp256k1). FROST is explicitly rejected because its Schnorr output is not recoverable by `secp256k1_recover`.

**Curve and encoding constraints:**

- Curve: secp256k1 only. No other curve is accepted by the host function.
- Scalar encoding: big-endian, fixed 32 bytes for both `r` and `s` (left-pad with zero bytes if the library emits shorter values).
- Point encoding: SEC-1. The aggregate pubkey registered on-chain may be compressed (33 bytes, `0x02`/`0x03` prefix) or uncompressed (65 bytes, `0x04` prefix); the contract normalises both before comparison.
- Recovery id: `v` is derived by trial recovery against the aggregate pubkey (§2b step 3), never trusted from the library output.
- Low-`s` normalisation: signatures MUST be normalised to low-`s` (`s ≤ n/2`) before submission so that `v` is unambiguous.

---

## 3. Distributed key generation (DKG) ceremony

The aggregate public key is produced by a t-of-n DKG ceremony in which no single party ever holds the full private key. The ceremony is implemented by the `ceremony` CLI (see §3.3) and follows the GG20 key-generation rounds.

### 3.1 Round structure

| Round | Message | Purpose |
|-------|---------|---------|
| 0 | `Commitment` | Each participant commits to a random polynomial and broadcasts a hash of its coefficient commitments. |
| 1 | `Share` | Each participant reveals its coefficient commitments and sends a Feldman-verifiable share to every other participant. |
| 2 | `Verify` | Each participant verifies every received share against the sender's commitments and broadcasts an accusation for any invalid share. |
| 3 | `Finalize` | Participants agree on the qualified set, compute their key share, and derive the aggregate public key. |

### 3.2 Transcript hashing and proof-of-correct-generation

Every message is appended to a running transcript. The transcript hash is:

```
H_0 = SHA-256("threshold-dkg-v1" || session_id || n_le || t_le)
H_i = SHA-256(H_{i-1} || round_le || sender_id_le || message_bytes)
```

The final `H_final` is the **transcript hash**. The **proof-of-correct-generation** output is:

```
proof = {
  session_id,
  n, t,
  aggregate_pubkey,          // SEC-1 encoded
  participant_ids: [id_0 … id_{n-1}],
  qualified_ids:   [id_0 … id_{t-1}],
  transcript_hash: H_final,
  per_participant_commitment_hashes: [h_0 … h_{n-1}],
}
```

This proof is published alongside the aggregate pubkey and is referenced by the contract-side registration flow (`set_aggregate_service_pubkey`) so that any observer can re-derive `H_final` from the published transcript and confirm the key was generated by the claimed participant set.

### 3.3 Ceremony CLI

The `ceremony` CLI drives the rounds above. It is round-based and stateless between invocations: each invocation reads the current transcript file, processes one round's inbound messages, and writes the updated transcript plus outbound messages.

```
ceremony init    --session <id> --n <N> --t <T> --out <dir>
ceremony round0  --session <id> --participant <i> --in <dir> --out <dir>
ceremony round1  --session <id> --participant <i> --in <dir> --out <dir>
ceremony round2  --session <id> --participant <i> --in <dir> --out <dir>
ceremony round3  --session <id> --participant <i> --in <dir> --out <dir>
ceremony verify  --session <id> --transcript <file> --proof <file>
ceremony rotate  --session <id> --n <N> --t <T> --out <dir>
```

**Participant verification.** In round 2 each participant verifies every received share against the sender's Feldman commitments. A share that fails verification is not silently dropped: the participant emits an `Accusation` message naming the sender.

**Abort handling.** The ceremony aborts (non-zero exit, no proof emitted) when:

- Fewer than `t` participants complete round 1 (insufficient qualified set).
- Any participant is accused by more than `t - 1` peers (unrecoverable misbehaviour).
- The transcript hash fails to reproduce during `ceremony verify`.

On abort the CLI writes an `abort.json` recording the round, the offending participant(s), and the transcript hash at the point of failure, so the incident can be audited without re-running the ceremony.

### 3.4 Rotation

`ceremony rotate` runs a fresh DKG with a new `session_id` and a new participant set, producing a new aggregate pubkey and proof. The old key remains valid on-chain until the admin calls `set_aggregate_service_pubkey` with the new key; see `docs/key-rotation-runbook.md` for the operational sequence.

---

## 4. Off-chain signing workflow

```
1. Each participant i holds a key share k_i produced during the DKG ceremony.
2. The t coordinators run the GG20 signing protocol on the payload commitment C:
     C = SHA-256(wallet_bytes || pair_bytes || score_le || flags || ts_le
                 || confidence_le || model_version_le || contract_addr || network_id)
   (same preimage layout as ScoreAttestation — see docs/attestation-spec.md)
3. The protocol produces (r, s). Append the recovery id v ∈ {0, 1} by
   trying secp256k1_recover(C, r, s, 0) and checking whether the result
   matches the aggregate public key; if not, v = 1.
4. Concatenate threshold_sig = r (32 bytes) || s (32 bytes) || v (1 byte).
5. Call submit_score(..., threshold_attestation = Some(ThresholdAttestation {
       commitment: C,
       threshold_sig,
       participating_signers: [addr_i1, addr_i2, …, addr_it],
   })).

### Key registration

After the DKG ceremony, the group's aggregate public key (an uncompressed 65-byte or compressed 33-byte SEC-1 secp256k1 point) is registered on-chain by the admin:

```bash
soroban contract invoke ... -- set_aggregate_service_pubkey \
  --admin_signers '[]' \
  --pubkey '<hex-encoded 33 or 65 byte SEC-1 pubkey>'
```yaml

The key can be rotated at any time by calling `set_aggregate_service_pubkey` again with the new key (requires admin authorization). There is no way to unset it once registered — consistent with the `set_service_pubkey` invariant.

---

## 5. On-chain verification

When `submit_score` receives a `threshold_attestation: Some(ta)`:

1. **Aggregate pubkey check** — fails immediately with `AggregatePubkeyNotSet` if no key has been registered.
2. **Signer membership** — all addresses in `ta.participating_signers` must be in the registered service set (if any). Fails with `ThresholdSignerNotInSet` otherwise.
3. **Threshold count** — `participating_signers.len()` must meet the configured `ServiceThreshold`. Fails with `InsufficientThresholdSigners` otherwise.
4. **Commitment recomputation** — the contract recomputes `C` from the call's actual arguments and compares it with `ta.commitment`. Fails with `InvalidThresholdSignature` on mismatch.
5. **Signature recovery** — `secp256k1_recover(C, r || s, v)` is called. The recovered point is compared against the registered aggregate pubkey (compressed or uncompressed). Fails with `InvalidThresholdSignature` on mismatch.

No `require_auth` call is ever made in the threshold path — the signature is the sole authorization proof.

---

## 6. Interaction with the ordinary attestation path

- If `threshold_attestation` is `Some`, the threshold path runs and the `attestation` parameter is ignored entirely.
- If `threshold_attestation` is `None`, the existing paths run unchanged:
  - If a service set is configured → M-of-N `require_auth` path.
  - Otherwise → legacy single-service `require_auth` path.
  - If `set_service_pubkey` has been called → ordinary `ScoreAttestation` is required.

This means upgrading to threshold signatures is a drop-in change: existing callers can pass `threshold_attestation: None` indefinitely, and the new path is only activated when both the aggregate pubkey has been registered and the caller passes a `ThresholdAttestation`.

---

## 7. Storage

| Key | Type | Description |
|-----|------|-------------|
| `AggregatePubKey` | `Bytes` (33 or 65) | Aggregate secp256k1 public key for the threshold group. Instance storage. |

---

## 8. Events

| Event topic | Data | When |
|-------------|------|------|
| `("agg_pk",)` | `pubkey: Bytes` | `set_aggregate_service_pubkey` succeeds. |

---

## 9. New error codes

| Code | Name | Context |
|------|------|---------|
| 52 | `AggregatePubkeyNotSet` | Threshold path attempted but no aggregate pubkey registered. |
| 53 | `InvalidThresholdSignature` | Commitment mismatch, bad recovery id, or recovered key ≠ aggregate key. |
| 54 | `ThresholdSignerNotInSet` | A `participating_signers` entry is not in the service set. |
| 55 | `InsufficientThresholdSigners` | `participating_signers.len()` < `ServiceThreshold`. |

---

## 10. Security notes

- **Key rotation:** rotate the aggregate key after any suspected share compromise by calling `set_aggregate_service_pubkey` with the new post-rotation key.
- **Replay protection:** the commitment preimage includes the contract address and network ID, so a valid threshold signature cannot be replayed on a different contract or network.
- **No `participating_signers` forgery:** the on-chain check validates membership but does NOT call `require_auth`. An attacker who can forge the threshold signature would need to break secp256k1 ECDSA, which is equivalent to breaking Bitcoin's signature scheme.
- **1-of-1 degenerate case:** when `ServiceThreshold = 1` and a single-signer "aggregate" key is registered, the threshold path degenerates to a standard single-key ECDSA attestation — mathematically sound but semantically equivalent to `set_service_pubkey`.
- **Ceremony integrity:** the transcript hash binds every round message to the session, so a participant cannot substitute a different polynomial or share set after the fact without changing `H_final`. The published proof-of-correct-generation must be archived with the aggregate pubkey; a registration without a matching proof should be treated as suspect.
