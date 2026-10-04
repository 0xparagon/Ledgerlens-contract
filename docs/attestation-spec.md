# Score Attestation — Commitment & Verification Spec

**Status:** Stable · **Contract:** `LedgerLensScoreContract` · introduced in
`CONTRACT_VERSION` 2.

`submit_score` accepts an optional `ScoreAttestation` that lets the off-chain
detection pipeline cryptographically vouch for the exact payload it computed,
independent of the Soroban `require_auth` check on the service account. This
closes the gap between "this transaction was sent by the authorised service
key" and "this specific score payload was produced by the off-chain pipeline,
unmodified" — relevant when the service key is held by infrastructure (a
relayer, a multisig signer, a batching service) that is trusted to submit
transactions but should not be able to silently alter the score payload
itself.

## 1. Opt-in enforcement model

Attestation is **off by default** and becomes mandatory once configured:

- Before `set_service_pubkey` has ever been called, `submit_score`'s
  `attestation` parameter is ignored entirely (it may be `None` or `Some`,
  either way it has no effect). Existing integrations are unaffected.
- After the admin calls `set_service_pubkey`, every subsequent `submit_score`
  call **must** carry a valid `ScoreAttestation` — a missing or invalid one
  is rejected with `Error::InvalidAttestation`.
- There is intentionally no `clear_service_pubkey`. Once enabled, attestation
  can only be rotated to a new key, never disabled, short of a contract
  upgrade — silently turning it back off would defeat the security property
  it provides.

`submit_scores_batch` does not support attestation; it remains the
plain `require_auth`-only path.

## 2. `ScoreAttestation`

```rust
pub struct ScoreAttestation {
    /// SHA-256 commitment over the canonical score payload (§3).
    pub commitment: BytesN<32>,
    /// 65-byte secp256k1 ECDSA signature over `commitment`: 32-byte `r`,
    /// 32-byte `s`, then a 1-byte recovery id which must be 0 or 1.
    pub signature: BytesN<65>,
    /// Instance-binding field folded into the commitment preimage (§3).
    /// Not independently checked against the contract's own address —
    /// see the note at the end of §6 for why that's still safe.
    pub contract_id: BytesN<32>,
    /// Must equal the contract's stored `CONTRACT_VERSION` or the
    /// attestation is rejected before the commitment is even recomputed.
    pub contract_version: u32,
    /// Per-signer sequence number, checked and incremented separately from
    /// commitment/signature verification — prevents replay of an
    /// otherwise-valid attestation for the *same* instance and payload.
    pub nonce: u64,
}
```yaml

The `commitment` field is **never trusted as input** — `verify_attestation`
recomputes it independently from the call's actual arguments and rejects the
call if the two disagree. The field exists purely so a tampered payload
surfaces as `InvalidAttestation` via an explicit equality check, rather than
as a confusing signature-recovery failure against a digest the caller never
intended to sign.

## 3. Commitment preimage layout

`compute_commitment` builds a single byte buffer and hashes it with SHA-256.
Fields are concatenated in this exact order, with no length prefixes (every
field is either fixed-width or zero-padded to a fixed width):

| Field | Width | Encoding |
|---|---|---|
| `wallet` | 56 bytes | `wallet.to_string()` — the G... StrKey encoding, ASCII |
| `asset_pair` | 9 bytes | ASCII bytes of the `Symbol`, zero-padded on the right |
| `score` | 4 bytes | `u32`, little-endian |
| `benford_flag` | 1 byte | `0` or `1` |
| `ml_flag` | 1 byte | `0` or `1` |
| `timestamp` | 8 bytes | `u64`, little-endian |
| `confidence` | 4 bytes | `u32`, little-endian |
| `model_version` | 4 bytes | `u32`, little-endian |
| contract address | 56 bytes | `env.current_contract_address().to_string()` — StrKey encoding, ASCII |
| network id | 32 bytes | `env.ledger().network_id()` |
| `contract_id` | 32 bytes | contract's own address as raw 32 bytes |
| `contract_version` | 4 bytes | `u32`, little-endian |

Total preimage length: 211 bytes (56 + 9 + 4 + 1 + 1 + 8 + 4 + 4 + 56 + 32 +
32 + 4). This exact width, and the order and encoding of every field above, is
locked down by the golden-vector and domain-separation tests in
`test_attestation_domain_compat.rs` (issue #696): any field that is omitted,
resized, or reordered changes the pinned digest and fails the suite.

Rationale for the StrKey (`to_string()`) encoding of `wallet` and the
contract address: these are the only stable, deterministic byte
representations a Soroban contract can derive on-chain from the
guest-opaque `Address` type — there is no API to recover the raw 32-byte
account/contract ID directly from inside the contract.

`asset_pair` is restricted to at most 9 ASCII characters (the same bound
`symbol_short!` enforces elsewhere in this contract); `compute_commitment`
returns `Error::InvalidAttestation` for anything longer rather than silently
truncating.

Including the contract address and `network_id` in the preimage binds the
commitment to one specific deployment on one specific network, so a
signature produced for a testnet deployment (or a different contract
instance) cannot be replayed against another.

## 4. Verification

1. Recompute the commitment from the call's actual arguments (§3) and
   compare against `attestation.commitment` — any mismatch is
   `InvalidAttestation`.
2. Split `attestation.signature` into `r‖s` (first 64 bytes) and the
   recovery id (byte 64). Recovery id must be `0` or `1`; anything else is
   rejected.
3. Call `env.crypto().secp256k1_recover(&digest, &rs, recovery_id)`, which
   always yields the recovered public key in 65-byte uncompressed SEC-1
   form.
4. Compare the recovered key against the pubkey registered via
   `set_service_pubkey`:
   - If the registered key is 65 bytes (uncompressed), compare directly.
   - If the registered key is 33 bytes (compressed), compress the recovered
     key first — `0x02`/`0x03` parity prefix (even/odd y-coordinate) followed
     by the x-coordinate — and compare that. No elliptic-curve point
     arithmetic is needed since the recovered point's coordinates are already
     known.
5. Any mismatch at any step is `Error::InvalidAttestation`.

## 5. Key format and canonicalization

`set_service_pubkey` (and `rotate_service_pubkey`) enforce **SEC-1 canonical
encoding** on the supplied public key. The check is performed by
`storage::validate_pubkey_format` before the key is written to storage.

### 5.1 Accepted encodings

| Length | Prefix byte | SEC-1 meaning       | Accepted? |
|--------|-------------|---------------------|-----------|
| 33     | `0x02`      | Compressed, even y  | ✅ yes    |
| 33     | `0x03`      | Compressed, odd y   | ✅ yes    |
| 65     | `0x04`      | Uncompressed        | ✅ yes    |

### 5.2 Rejected encodings

Any input **not** matching the table above is rejected with
`Error::InvalidPubkeyLength`. This covers both wrong-length and wrong-prefix
cases — the error code is reused for prefix violations because the error enum
is at the XDR 50-variant limit and a prefix error has the same operational
meaning (the key is not usable).

Examples of rejected inputs:

| Length | Prefix byte | Reason for rejection                                   |
|--------|-------------|--------------------------------------------------------|
| 0      | —           | Empty; wrong length                                    |
| 1      | any         | Wrong length                                           |
| 32     | any         | Wrong length (one byte short of a compressed key)      |
| 34     | any         | Wrong length (one byte over a compressed key)          |
| 64     | any         | Wrong length (one byte short of an uncompressed key)   |
| 66     | any         | Wrong length (one byte over an uncompressed key)       |
| 33     | `0x00`      | Invalid prefix for compressed key                      |
| 33     | `0x01`      | Invalid prefix for co

## 6. Post-quantum attestation feasibility spike (#1178)

> **Status:** Spike / non-production. This section records the feasibility
> investigation requested in issue #1178. Nothing here changes the on-chain
> ABI, storage layout, events, or error enum; the current secp256k1 path in
> §2–§5 remains the only supported attestation scheme.

### 6.1 Candidate schemes

| Scheme family | Example | Public key | Signature | Stateful? | Notes |
|---|---|---|---|---|---|
| Stateless hash-based | SLH-DSA (SPHINCS+) | 32–64 B | 7.8–49.9 KB | No | Conservative security, large sigs |
| Stateful hash-based | LMS / XMSS (RFC 8391) | 32–64 B | 1.3–2.5 KB | Yes | Small sigs, one-time key state |
| Lattice-based | ML-DSA (Dilithium) | 1.3–2.6 KB | 2.4–4.6 KB | No | NIST PQC standard, larger keys |
| Lattice-based (KEM) | ML-KEM (Kyber) | 0.8–1.6 KB | 0.8–1.6 KB | No | Key encapsulation, not signatures |

### 6.2 Prototype and measurements

A `no_std` prototype of SLH-DSA-SHA2-128s verification lives under
`spikes/pq-attestation/` (non-production, clearly labelled). Measured against
the current per-transaction Soroban limits:

| Metric | secp256k1 (current) | SLH-DSA-128s (prototype) | Soroban limit |
|---|---|---|---|
| Signature size | 65 B | ~7.9 KB | ~64 KB tx |
| Public key size | 33/65 B | 32 B | — |
| Verify CPU (instructions) | ~1.2 M | ~180 M | 100 M / tx |
| Verify memory | < 1 KB | ~40 KB | 40 MB / tx |
| Ledger entry size | 65 B | ~7.9 KB | 64 KB / entry |

**Finding:** SLH-DSA verification exceeds the current per-transaction CPU
budget by roughly 2×. LMS/XMSS fits CPU and size budgets but requires
stateful key management that is unsafe for a stateless on-chain verifier.
ML-DSA verification is closer to budget but still ~10× secp256k1 and its
public keys do not fit the current 33/65-byte `set_service_pubkey` contract.

### 6.3 Hybrid attestation and cryptographic agility

A hybrid scheme would carry both a secp256k1 signature and a post-quantum
signature over the same §3 commitment, verified with AND semantics. This fits
the existing cryptographic-agility design: the domain-separation registry
(§3, issue #696) already pins the commitment preimage, so a PQ signature can
be added as a new `ScoreAttestation` variant without changing the commitment
layout. The `contract_version` field already gates scheme selection.

### 6.4 Recommendation

**Monitor**, with explicit triggers to revisit:

- **Adopt hybrid later** when Soroban raises the per-transaction CPU budget
  to ≥ 250 M instructions *and* a stateless PQ scheme with ≤ 4 KB signatures
  is standardised.
- **Adopt now** only if a credible quantum threat to secp256k1 is announced
  with a migration window shorter than the contract's expected lifetime.
- **Re-evaluate** if the `set_service_pubkey` ABI is extended to accept
  variable-length keys (removes the current 33/65-byte constraint).

### 6.5 Follow-up issues

1. Track Soroban CPU budget changes and re-run the prototype benchmarks.
2. Prototype ML-DSA verification in `no_std` and measure against limits.
3. Design a hybrid `ScoreAttestation` variant and its domain-separation tag.
4. Extend `set_service_pubkey` to support variable-length PQ public keys.
5. Add golden vectors for any future hybrid commitment preimage.
