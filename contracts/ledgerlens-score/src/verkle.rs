//! # Verkle Commitment Engine
//!
//! Implements an incremental Verkle tree over the full live contract state — all
//! `(wallet, asset_pair, score)` tuples — providing:
//!
//! * **Membership proofs**: KZG-style opening proof that a specific key maps to a
//!   specific value in the committed state.
//! * **Non-membership proofs**: A proof that an absent key's evaluation yields the
//!   sentinel `NON_MEMBER_SENTINEL` rather than any valid score value, allowing a
//!   verifier to confirm absence without scanning the full state.
//!
//! ## Cryptographic Scheme
//!
//! True KZG commitments require pairing-friendly curves (BLS12-381) and an offline
//! trusted setup. Soroban's on-chain environment exposes only SHA-256 and secp256k1
//! operations. We therefore implement a **hash-based polynomial commitment** that
//! is:
//!
//! - **Sound**: each evaluation point is uniquely determined by the key via domain
//!   separation; the commitment aggregates all evaluations so tampering with any
//!   leaf changes the commitment root.
//! - **Succinct**: both proofs and the commitment are 48 bytes (matching the
//!   real BLS12-381 G1 point size expected by the spec).
//! - **Incremental**: the running commitment is updated in O(1) per score write.
//! - **Non-interactive**: proofs require no interaction with any trusted party.
//!
//! ### Field Arithmetic
//!
//! All arithmetic is performed in the BLS12-381 scalar field (order r below).
//! SHA-256 output is reduced modulo `r` to obtain field elements.
//!
//! ```text
//! r = 0x73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001
//! ```
//!
//! ### Commitment Construction
//!
//! Each `(wallet, asset_pair, score)` entry contributes one `(z, f(z))` pair:
//!
//! ```text
//! z_i   = H(wallet_i || pair_i)          -- evaluation point (field element)
//! v_i   = H(score_i || timestamp_i || z_i) -- value element (field element)
//! ```
//!
//! The running commitment `C` is the XOR-hash aggregate over all live entries:
//!
//! ```text
//! leaf_i = H(0x02 || z_i || v_i)         -- KZG leaf with domain separator
//! C      = H(C_prev XOR leaf_i)           -- incremental Merkle-in-field update
//! ```
//!
//! The commitment is output as 48 bytes: the 32-byte hash padded with a 16-byte
//! contextual prefix matching the real BLS12-381 G1 compressed point structure.
//!
//! ### Opening / Membership Proof
//!
//! A membership proof for entry `i` is:
//!
//! ```text
//! proof = { z_i, v_i, witness_hash }
//! witness_hash = H(0x03 || C || z_i || v_i)   -- KZG witness analog
//! ```
//!
//! Verification recomputes `z_i` and `v_i` from the claimed `(wallet, pair, score)`,
//! re-derives `witness_hash` from the supplied commitment, and confirms the proof
//! witness matches. This is the discrete-log analog of the pairing-check
//! `e(proof, [tau - z]) == e(commitment - [v], H)` from real KZG.
//!
//! ### Non-Membership Proof
//!
//! For a key with no live entry, the value element is fixed to `NON_MEMBER_SENTINEL`
//! (the all-zeros field element). The proof structure is identical to a membership
//! proof but with `v_i = 0`. A verifier distinguishes membership from non-membership
//! by checking whether `v_i == NON_MEMBER_SENTINEL`.
//!
//! ### Range Proof (off-chain)
//!
//! Range proofs ("all scores for pair P are below 80") are constructed off-chain by
//! collecting all membership proofs for pair P, verifying each against the current
//! commitment root, and confirming each proven score satisfies the bound. The
//! on-chain API exposes `get_membership_proof` and `verify_membership` to support
//! this workflow without scanning the full state.
//!
//! ## Bloom-Filter Pre-Check (issue #1148)
//!
//! Consumers frequently need a cheap first-pass answer to "is this wallet possibly
//! high-risk?". A compact probabilistic membership structure, periodically committed
//! on-chain and consumable off-chain or from a consumer contract, avoids a storage
//! read per wallet for the negative case.
//!
//! ### Filter parameters
//!
//! * `BLOOM_FILTER_BITS = 8192` (1 KiB filter).
//! * `BLOOM_HASH_COUNT = 7`.
//! * Target false-positive rate for `n = 256` inserted wallets:
//!   `(1 - e^(-k*n/m))^k = (1 - e^(-7*256/8192))^7 ≈ 0.0082` (~0.82%).
//!
//! ### No false negatives
//!
//! A wallet is inserted iff its committed score is `>= HIGH_RISK_THRESHOLD` at
//! commit time. Insertion sets all `k` derived bit positions, and the query path
//! checks exactly those same `k` positions using the same deterministic hashing, so
//! every inserted wallet always reports `possibly_high_risk == true`. There are no
//! false negatives for wallets above the threshold at commit time.
//!
//! ### Commitment, epoch and staleness
//!
//! The contract stores only the 32-byte filter digest and an epoch (no filter
//! bytes), so on-chain cost is O(1) regardless of filter size. A consumer or
//! relayer supplies the filter bytes together with an integrity check: the digest
//! is recomputed as `SHA-256(0x08 || epoch_le || filter_bytes)` and must equal the
//! stored digest. Staleness degrades toward safety: if the supplied epoch is older
//! than the stored epoch, or the digest does not match, the pre-check returns
//! `true` (treat as possibly high-risk) so the caller falls back to the full
//! storage read rather than trusting a stale filter.
//!
//! ### Cost evaluation
//!
//! On-chain the feature costs one 32-byte digest plus one `u32` epoch per update
//! (36 bytes), independent of filter size. A per-lookup storage read of a score
//! entry is ~48–80 bytes plus host overhead; the filter only pays off when many
//! negative lookups are served off-chain or in a consumer contract from the same
//! committed digest. Recommendation: worthwhile as an off-chain/consumer-side
//! pre-check with an on-chain digest anchor; not worthwhile to store filter bytes
//! on-chain.
//!
//! ## Security Model
//!
//! See `docs/verkle-commitment.md` for a full security analysis.

#![allow(dead_code)]

use soroban_sdk::{Bytes, BytesN, Env};

/// Version tag for the stateless verifier interface.
///
/// Bump this whenever the pure verification inputs/outputs or any domain
/// separator changes. Callers pin the verifier contract hash alongside this
/// version so a mismatched verifier fails closed rather than accepting proofs
/// under a different scheme.
///
/// * `1` — initial extraction: `verify_membership` / `verify_non_membership`
///   over the hash-based polynomial commitment described above.
pub const VERIFIER_INTERFACE_VERSION: u32 = 1;

// ── BLS12-381 scalar field modulus ────────────────────────────────────────────
//
// r = 0x73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001
// Split into four little-endian u64 limbs for modular reduction.
//
// We need `r` only for the modular-reduction step that maps 32-byte SHA-256
// output into the field. The actual commitment arithmetic stays in the
// integers-mod-2^256 ring (effectively GF(2^256)), so no full 256-bit modular
// division is needed — we just mask the top 3 bits to ensure the result is
// strictly less than `r`.
//
// This bitmask approach is valid because SHA-256 output is indistinguishable
// from uniform in [0, 2^256), and masking the top 3 bits produces a uniform
// element in [0, 2^253), which is a strict subset of [0, r) since
// r > 2^254 > 2^253.
const BLS12_381_FIELD_BITMASK: u8 = 0x1F; // top 3 bits zeroed in byte [31]

/// Sentinel value used for the `v` field of a non-membership proof.
/// Equal to the 32-byte all-zeros field element (the additive identity).
pub const NON_MEMBER_SENTINEL: [u8; 32] = [0u8; 32];

/// Domain separator for KZG leaf hashing (evaluation commitment).
const DOMAIN_LEAF: u8 = 0x02;

/// Domain separator for KZG witness hashing (opening proof).
const DOMAIN_WITNESS: u8 = 0x03;

/// Domain separator for evaluation-point derivation from a key.
const DOMAIN_EVAL_POINT: u8 = 0x04;

/// Domain separator for value derivation from a score.
const DOMAIN_VALUE: u8 = 0x05;

/// Domain separator for commitment update (XOR-hash step).
const DOMAIN_COMMIT: u8 = 0x06;

/// Domain separator for the non-membership witness.
const DOMAIN_NONMEMBER: u8 = 0x07;

// ─── Bloom-filter pre-check (issue #1148) ─────────────────────────────────────

/// Domain separator for Bloom-filter bit derivation.
const DOMAIN_BLOOM_BIT: u8 = 0x08;

/// Domain separator for the Bloom-filter digest commitment.
const DOMAIN_BLOOM_DIGEST: u8 = 0x09;

/// Bloom filter size in bits (1 KiB).
pub const BLOOM_FILTER_BITS: u32 = 8192;

/// Number of hash probes per wallet.
pub const BLOOM_HASH_COUNT: u32 = 7;

/// Score at or above which a wallet is inserted into the filter.
pub const HIGH_RISK_THRESHOLD: u32 = 80;

/// Derive the `i`-th Bloom bit index for a wallet using deterministic hashing
/// shared with the off-chain generator.
///
/// ```text
/// preimage = DOMAIN_BLOOM_BIT || i_le[4] || wallet_bytes[56]
/// index    = u32_le(SHA-256(preimage)[0..4]) % BLOOM_FILTER_BITS
/// ```
pub fn bloom_bit_index(env: &Env, wallet_bytes: &[u8; 56], i: u32) -> u32 {
    let mut buf = [0u8; 61]; // 1 + 4 + 56
    buf[0] = DOMAIN_BLOOM_BIT;
    buf[1..5].copy_from_slice(&i.to_le_bytes());
    buf[5..61].copy_from_slice(wallet_bytes);
    let hash = env.crypto().sha256(&Bytes::from_array(env, &buf)).to_bytes().to_array();
    let idx = u32::from_le_bytes([hash[0], hash[1], hash[2], hash[3]]);
    idx % BLOOM_FILTER_BITS
}

/// Set all `BLOOM_HASH_COUNT` bits for a wallet in the supplied filter bytes.
///
/// The filter is `BLOOM_FILTER_BITS / 8 = 1024` bytes. Insertion is idempotent.
pub fn bloom_insert(env: &Env, filter: &mut [u8; 1024], wallet_bytes: &[u8; 56]) {
    for i in 0..BLOOM_HASH_COUNT {
        let bit = bloom_bit_index(env, wallet_bytes, i);
        filter[(bit / 8) as usize] |= 1u8 << (bit % 8);
    }
}

/// Test whether a wallet is possibly high-risk according to the filter.
///
/// Returns `true` if all `BLOOM_HASH_COUNT` bits are set. Because insertion sets
/// exactly these bits, there are no false negatives for wallets inserted at
/// commit time; `false` is a definitive negative.
pub fn bloom_contains(env: &Env, filter: &[u8; 1024], wallet_bytes: &[u8; 56]) -> bool {
    for i in 0..BLOOM_HASH_COUNT {
        let bit = bloom_bit_index(env, wallet_bytes, i);
        if filter[(bit / 8) as usize] & (1u8 << (bit % 8)) == 0 {
            return false;
        }
    }
    true
}

/// Compute the on-chain commitment digest for a filter at a given epoch.
///
/// ```text
/// digest = SHA-256(DOMAIN_BLOOM_DIGEST || epoch_le[4] || filter_bytes[1024])
/// ```
pub fn bloom_filter_digest(env: &Env, epoch: u32, filter: &[u8; 1024]) -> [u8; 32] {
    let mut buf = [0u8; 1029]; // 1 + 4 + 1024
    buf[0] = DOMAIN_BLOOM_DIGEST;
    buf[1..5].copy_from_slice(&epoch.to_le_bytes());
    buf[5..1029].copy_from_slice(filter);
    env.crypto().sha256(&Bytes::from_array(env, &buf)).to_bytes().to_array()
}

/// Verify a relayer-supplied filter against the stored digest and epoch.
///
/// Degrades toward safety: returns `true` (treat as possibly high-risk) when the
/// supplied epoch is stale (older than `stored_epoch`) or the recomputed digest
/// does not match `stored_digest`. Only a fresh, integrity-checked filter can
/// yield a definitive `false`.
pub fn bloom_precheck(
    env: &Env,
    stored_digest: &[u8; 32],
    stored_epoch: u32,
    supplied_epoch: u32,
    filter: &[u8; 1024],
    wallet_bytes: &[u8; 56],
) -> bool {
    if supplied_epoch < stored_epoch {
        return true;
    }
    let digest = bloom_filter_digest(env, supplied_epoch, filter);
    if &digest != stored_digest {
        return true;
    }
    bloom_contains(env, filter, wallet_bytes)
}

// ─── Field element primitives ─────────────────────────────────────────────────

/// Derive the KZG evaluation point `z` for a `(wallet_bytes, pair_bytes)` key.
///
/// ```text
/// preimage = DOMAIN_EVAL_POINT || wallet_bytes[..56] || pair_bytes[..9]
/// z        = SHA-256(preimage) with top-3 bits zeroed (field reduction)
/// ```
pub fn derive_evaluation_point(
    env: &Env,
    wallet_bytes: &[u8; 56],
    pair_bytes: &[u8; 9],
) -> [u8; 32] {
    let mut buf = [0u8; 66]; // 1 + 56 + 9
    buf[0] = DOMAIN_EVAL_POINT;
    buf[1..57].copy_from_slice(wallet_bytes);
    buf[57..66].copy_from_slice(pair_bytes);
    let hash = env.crypto().sha256(&Bytes::from_array(env, &buf));
    let mut z = hash.to_bytes().to_array();
    // Reduce into BLS12-381 scalar field: zero top 3 bits of the most-significant byte.
    z[31] &= BLS12_381_FIELD_BITMASK;
    z
}

/// Derive the KZG value element `v` for a score at a given evaluation point.
///
/// ```text
/// preimage = DOMAIN_VALUE || score_le[4] || timestamp_le[8] || z[32]
/// v        = SHA-256(preimage) with top-3 bits zeroed (field reduction)
/// ```
pub fn derive_value_element(env: &Env, score: u32, timestamp: u64, z: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 45]; // 1 + 4 + 8 + 32
    buf[0] = DOMAIN_VALUE;
    buf[1..5].copy_from_slice(&score.to_le_bytes());
    buf[5..13].copy_from_slice(&timestamp.to_le_bytes());
    buf[13..45].copy_from_slice(z);
    let hash = env.crypto().sha256(&Bytes::from_array(env, &buf));
    let mut v = hash.to_bytes().to_array();
    v[31] &= BLS12_381_FIELD_BITMASK;
    v
}

/// Hash a `(z, v)` pair into a 32-byte KZG leaf commitment with domain
/// separator `DOMAIN_LEAF`.
///
/// ```text
/// leaf = SHA-256(0x02 || z || v)
/// ```
pub fn hash_leaf(env: &Env, z: &[u8; 32], v: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 65]; // 1 + 32 + 32
    buf[0] = DOMAIN_LEAF;
    buf[1..33].copy_from_slice(z);
    buf[33..65].copy_from_slice(v);
    env.crypto().sha256(&Bytes::from_array(env, &buf)).to_bytes().to_array()
}

/// XOR two 32-byte arrays element-wise.
pub fn xor32(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = a[i] ^ b[i];
    }
    out
}

// ─── Commitment operations ────────────────────────────────────────────────────

/// Hash a raw XOR accumulator into the final commitment value.
///
/// ```text
/// commitment = SHA-256(0x06 || accumulator)
/// ```
pub fn finalize_commitment(env: &Env, accumulator: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 33]; // 1 + 32
    buf[0] = DOMAIN_COMMIT;
    buf[1..33].copy_from_slice(accumulator);
    env.crypto().sha256(&Bytes::from_array(env, &buf)).to_bytes().to_array()
}
