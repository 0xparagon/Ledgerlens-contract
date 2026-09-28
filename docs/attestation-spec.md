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
| 33     | `0x01`      | Invalid prefix for compressed key                      |
| 65     | `0x02`      | Invalid prefix for uncompressed key                    |
| 65     | `0x03`      | Invalid prefix for uncompressed key                    |

## 6. Portable signed score credentials

A wallet owner sometimes needs to present their current risk standing to a
party that cannot query the chain directly (a custodian, an exchange, an
off-chain service). A **score credential** is a compact, signed statement of
a score snapshot that can be verified against the on-chain state or against
the service key, with a defined mapping to a standard credential envelope.

### 6.1 Credential fields

```rust
pub struct ScoreCredential {
    /// Holder the credential is bound to (StrKey G.../C... encoding).
    pub subject: Address,
    /// Asset pair the score applies to (≤ 9 ASCII chars).
    pub asset_pair: Symbol,
    /// Score snapshot value, 0..=100.
    pub score: u32,
    /// Confidence of the snapshot, 0..=100.
    pub confidence: u32,
    /// Model version that produced the snapshot.
    pub model_version: u32,
    /// Ledger timestamp at which the credential was issued.
    pub issued_at: u64,
    /// Ledger timestamp after which the credential is no longer valid.
    pub expiry: u64,
    /// Monotonic revision of the stored score this credential pins.
    pub revision: u64,
}
```

`revision` is the per-`(subject, asset_pair)` counter incremented on every
`submit_score`; it lets a verifier distinguish a credential for the current
snapshot from one for a superseded historical snapshot.

### 6.2 Canonical signing bytes with domain separation

`compute_credential_digest` builds a single byte buffer and hashes it with
SHA-256. The buffer is prefixed with a fixed domain-separation tag so a
credential signature can never be confused with a `ScoreAttestation`
commitment (§3) or any other signature produced by the service key:

| Field | Width | Encoding |
|---|---|---|
| domain tag | 32 bytes | ASCII `"LedgerLens/score-credential/v1"` zero-padded on the right |
| `subject` | 56 bytes | `subject.to_string()` — StrKey encoding, ASCII |
| `asset_pair` | 9 bytes | ASCII bytes of the `Symbol`, zero-padded on the right |
| `score` | 4 bytes | `u32`, little-endian |
| `confidence` | 4 bytes | `u32`, little-endian |
| `model_version` | 4 bytes | `u32`, little-endian |
| `issued_at` | 8 bytes | `u64`, little-endian |
| `expiry` | 8 bytes | `u64`, little-endian |
| `revision` | 8 bytes | `u64`, little-endian |
| contract address | 56 bytes | `env.current_contract_address().to_string()` — StrKey encoding, ASCII |
| network id | 32 bytes | `env.ledger().network_id()` |

Total preimage length: 221 bytes (32 + 56 + 9 + 4 + 4 + 4 + 8 + 8 + 8 + 56 +
32). The domain tag is the first 32 bytes of the preimage, so a signature over
a credential digest can never be replayed as a `ScoreAttestation` commitment
(and vice versa) even if every other field coincides.

### 6.3 Off-chain verification

Given a credential and its 65-byte secp256k1 signature (`r‖s‖recovery_id`):

1. Recompute the digest from the credential fields (§6.2) using the expected
   contract address and network id for the deployment being verified against.
2. Recover the public key with `secp256k1_recover` and compare it against the
   service pubkey registered via `set_service_pubkey` (§5).
3. Reject if `now > expiry` (expired) or if `issued_at > now` (not yet valid).
4. Optionally query the contract's stored score for `(subject, asset_pair)`
   and reject if the stored `revision` does not equal the credential's
   `revision` (stale credential) — see §6.4 for the on-chain equivalent.

### 6.4 On-chain `verify_credential`

```rust
pub fn verify_credential(
    env: Env,
    credential: ScoreCredential,
    signature: BytesN<65>,
) -> Result<bool, Error>
```

`verify_credential` is a read-only entry point that:

1. Recomputes the credential digest (§6.2) and recovers the signer, comparing
   it against the registered service pubkey exactly as in §4 steps 2–4. A
   mismatch returns `Ok(false)` (not an error) so callers can branch on the
   result without try/catch.
2. Rejects with `Error::InvalidAttestation` if `env.ledger().timestamp()` is
   greater than `credential.expiry` or less than `credential.issued_at`.
3. Loads the stored score for `(credential.subject, credential.asset_pair)`.
   If the stored `revision` equals `credential.revision`, the credential
   matches the **current** stored state and the function returns `Ok(true)`.
4. If the stored `revision` is greater than `credential.revision`, the
   credential pins a **historical** snapshot. The contract returns
   `Ok(true)` only when the caller also supplies the historical snapshot via
   `get_score_at_revision(subject, asset_pair, revision)` and every field
   (`score`, `confidence`, `model_version`) matches the credential. Otherwise
   it returns `Ok(false)`.
5. If no stored score exists for the pair, returns `Ok(false)`.

### 6.5 W3C Verifiable Credentials envelope mapping

The credential maps onto the W3C Verifiable Credentials data model without
pulling any dependency into the contract — the envelope is produced and
consumed entirely off-chain by SDKs:

```json
{
  "@context": [
    "https://www.w3.org/2018/credentials/v1",
    "https://ledgerlens.example/credentials/score/v1"
  ],
  "type": ["VerifiableCredential", "LedgerLensScoreCredential"],
  "issuer": "did:stellar:<service-pubkey-strkey>",
  "issuanceDate": "2024-01-01T00:00:00Z",
  "expirationDate": "2024-02-01T00:00:00Z",
  "credentialSubject": {
    "id": "did:stellar:<subject-strkey>",
    "assetPair": "XLM/USDC",
    "score": 87,
    "confidence": 92,
    "modelVersion": 3,
    "revision": 41
  },
  "proof": {
    "type": "StellarSecp256k1Signature2024",
    "created": "2024-01-01T00:00:00Z",
    "proofPurpose": "assertionMethod",
    "verificationMethod": "did:stellar:<service-pubkey-strkey>#key-1",
    "signatureValue": "<base64 r‖s‖recovery_id>"
  }
}
```

`issuanceDate`/`expirationDate` are the RFC 3339 renderings of `issued_at`
and `expiry`; `credentialSubject.id` is the StrKey of `subject`. The
`proof.signatureValue` is the base64 of the same 65-byte signature verified
on-chain, so a verifier can check the envelope off-chain and, if desired,
re-check it on-chain via `verify_credential`.

### 6.6 Replay, expiry and holder-binding

- **Replay:** the domain tag (§6.2) plus the contract address and network id
  bind a credential to one deployment on one network. A credential issued for
  testnet cannot be replayed on mainnet, and a credential signature cannot be
  replayed as a `ScoreAttestation` commitment.
- **Expiry:** `expiry` is checked both off-chain (§6.3) and on-chain (§6.4);
  an expired credential is rejected even if the signature is valid.
- **Holder-binding:** `subject` is part of the signed digest, so a credential
  cannot be presented by a different holder without invalidating the
  signature. Verifiers MUST compare `credential.subject` against the
  presenter's authenticated identity before accepting the credential.
- **Revision pinning:** `revision` ties the credential to a specific stored
  snapshot, so a superseded credential cannot be passed off as current.

### 6.7 Privacy note

Presenting a score credential discloses the holder's `subject` (their Stellar
account), the `asset_pair`, the exact `score` and `confidence`, the
`model_version`, and the `revision` of the stored snapshot — and, because the
credential is signed by the service key, it also links the holder to that
service and to the contract deployment and network in the digest. A verifier
can therefore correlate the holder across every credential they present and
learn the precise risk score rather than a coarse band. Holders should prefer
presenting the narrowest credential that satisfies the verifier (a single
pair, a short expiry) and should be aware that a credential is not
unlinkable: it is a signed, holder-bound statement, not an anonymous proof.
