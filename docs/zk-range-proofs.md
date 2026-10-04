# Zero-Knowledge Range Proofs for Score Queries

LedgerLens implements a zero-knowledge (ZK) range proof scheme allowing third-party contracts to verify that a wallet's risk score is below a chosen threshold $T$ (i.e. $score < T$) without the LedgerLens score contract revealing the exact score value to the calling contract.

This is achieved using **Pedersen Commitments** and **Bulletproofs** verified on-chain via a SHA-256-based Fiat-Shamir heuristic.

---

## Cryptographic Design

### Pedersen Commitment

When the LedgerLens service submits a score $v \in [0, 100]$ to the contract, it can optionally submit a Pedersen commitment:
$$C = g^v \cdot h^r \pmod p$$
where:
*   $g$ and $h$ are independent generators on the Twisted Edwards Curve (Ed25519).
*   $v$ is the score.
*   $r$ is a randomly chosen blinding factor (scalar) in the scalar field of Ed25519 ($\mathbb{F}_p$ where $p = 2^{255}-19$).

This commitment is stored in the contract's persistent storage alongside the score.

### Proving $v < T$

To prove that the score $v$ satisfies the threshold $T$ (i.e. $v \le T - 1$), the prover shows that:
$$v' = T - 1 - v \ge 0$$
Since $v \in [0, 100]$ is already verified on-chain during score submission, $v' \ge 0$ is sufficient to guarantee $v < T$.

The verifier on-chain can compute a commitment $C'$ to $v'$ dynamically from the stored commitment $C$ and the threshold $T$:
$$C' = g^{T-1} \cdot C^{-1} = g^{T-1} \cdot (g^v h^r)^{-1} = g^{T-1-v} \cdot h^{-r}$$
The blinding factor for $C'$ is $r' = -r \pmod p$.

The prover then provides a Bulletproof range proof showing that the value in $C'$ (which is $v'$) lies in the range $[0, 2^8)$ (i.e. $0 \le v' \le 255$, which is satisfied since $0 \le v' \le 99$).

---

## Bulletproof Range Proof Protocol

The Bulletproof range proof demonstrates that $v' \in [0, 2^8)$ in zero-knowledge.

1.  **Bit Commitments**:
    The prover decomposes $v'$ into its binary representation $\vec{a}_L \in \{0, 1\}^8$.
    It computes $\vec{a}_R = \vec{a}_L - \vec{1}$.
    It commits to these vectors using:
    $$A = h^{\alpha} \cdot \vec{g}^{\vec{a}_L} \cdot \vec{h}^{\vec{a}_R}$$
    It commits to blinding vectors $\vec{s}_L, \vec{s}_R$:
    $$S = h^{\beta} \cdot \vec{g}^{\vec{s}_L} \cdot \vec{h}^{\vec{s}_R}$$
2.  **Fiat-Shamir Challenges**:
    The prover hashes the transcript to generate challenges $y, z$.
3.  **Polynomial Formulation**:
    The prover defines vector polynomials:
    $$\vec{l}(X) = (\vec{a}_L - z \cdot \vec{1}) + \vec{s}_L X$$
    $$\vec{r}(X) = \vec{y}^n \circ (\vec{a}_R + z \cdot \vec{1} + \vec{s}_R X) + z^2 \vec{2}^n$$
    The inner product $t(X) = \langle \vec{l}(X), \vec{r}(X) \rangle = t_0 + t_1 X + t_2 X^2$ is committed using:
    $$T_1 = g^{t_1} h^{\tau_1}$$
    $$T_2 = g^{t_2} h^{\tau_2}$$
    The verifier sends a challenge $x$.
4.  **Evaluations**:
    The prover evaluates:
    $$t_x = t(x)$$
    $$\tau_x = \tau_2 x^2 + \tau_1 x + z^2 r'$$
    $$\mu = \alpha + \beta x$$
5.  **Inner Product Argument**:
    A 3-round recursive inner product argument is run to prove that $t_x = \langle \vec{l}(x), \vec{r}(x) \rangle$ holds, compressing the vector opening down to $O(\log n)$ points.

---

## On-Chain Contract API

### `submit_score`
The score submission function accepts the commitment packed inside the `ScoreAttestationInput` struct to stay within Soroban's 10-parameter limit:

```rust
pub struct ScoreAttestationInput {
    pub attestation: MaybeScoreAttestation,
    pub threshold_attestation: MaybeThresholdAttestation,
    pub commitment: Option<Bytes>, // The Pedersen commitment C
}

pub fn submit_score(
    env: Env,
    signers: Vec<Address>,
    wallet: Address,
    asset_pair: Symbol,
    score: u32,
    benford_flag: bool,
    ml_flag: bool,
    timestamp: u64,
    confidence: u32,
    attestation_input: Option<ScoreAttestationInput>,
) -> Result<(), Error>;
```yaml

### `verify_score_range_proof`
Allows third-party verifiers to check that a score is below a threshold:
```rust
pub fn verify_score_range_proof(
    env: Env,
    wallet: Address,
    asset_pair: Symbol,
    commitment: BytesN<32>, // The claimed commitment C
    proof: Bytes,           // The 800-byte Bulletproof
    threshold: u32,         // The threshold T
) -> bool;

The function returns `true` if:
1.  A score entry exists for the `(wallet, asset_pair)` pair.
2.  The stored commitment matches the provided `commitment`.
3.  The Bulletproof demonstrates that $T - 1 - v \in [0, 2^8)$ under $C' = g^{T-1} C^{-1}$.

---

## Security Model

*   **Binding**: The Pedersen commitment is perfectly binding. The score reporter cannot open the commitment to any score other than the one submitted.
*   **Hiding**: The commitment is perfectly hiding. A calling contract learns nothing about the exact score value from the commitment itself.
*   **Completeness**: A prover who knows the correct score and blinding factor can always generate a valid range proof that verifies.
*   **Soundness**: An attacker cannot generate a valid range proof for a score that is equal to or greater than the threshold without solving the discrete logarithm problem or finding SHA-256 collisions.

---

## Feasibility Spike: Verifying ZKML Inference Proofs on Soroban (#1189)

This section is a feasibility spike for extending the range-proof approach above to **zero-knowledge machine-learning (ZKML) inference proofs**: letting the chain verify that a published score was produced by a committed model on committed inputs, rather than trusting the operator. It is a report + cost model, not a shipped verifier.

### 1. Proof-system survey and Soroban verifier costs

Soroban exposes no native pairing precompile; the only cryptographic primitives available to a contract are the host functions for **SHA-256**, **Ed25519 signature verification**, and **secp256k1 recovery** (plus the `bn254`/`bls12-381` host functions added for protocol 21+ where enabled). This constrains which verifiers are implementable:

| System | Curve / primitive | Verifier work | Implementable on Soroban? |
| --- | --- | --- | --- |
| Bulletproofs (current) | Ed25519, no pairing | $O(n)$ group ops, no pairings | **Yes** — already shipped in `zk_range_proof.rs` |
| Groth16 | BN254 / BLS12-381 pairings | 3 pairings + ~4 G1/G2 MSM | Only if `bn254`/`bls12-381` host fns are enabled; otherwise no |
| PLONK / Halo2 | BN254 / BLS12-381 pairings | ~2 pairings + larger MSM | Same caveat as Groth16 |
| STARKs (FRI) | Hash-only (no pairing) | $O(\log^2 n)$ hashes, large proof | **Yes** — pure SHA-256, but proof size dominates |
| Spartan / sum-check | Hash + one group | Hash-heavy, small group work | Plausible via SHA-256 transcript |
| zk-SNARK over Ed25519 (e.g. Bulletproof-style) | Ed25519, no pairing | $O(n)$ group ops | **Yes** — matches existing primitive set |

**Finding:** the only verifier families that are *unconditionally* implementable with today's Soroban host functions are the **hash-only (STARK/FRI, Spartan)** and **Ed25519-group (Bulletproof-style)** families. Pairing-based SNARKs (Groth16/PLONK) are only viable if the `bn254`/`bls12-381` host functions are enabled on the target network; this must be confirmed before committing to a pairing-based design.

### 2. Toy model and circuit-size estimate

Representative toy model: a **logistic scorer** over $d = 16$ quantized features with 8-bit fixed-point weights, i.e. $y = \sigma(\sum_{i=1}^{16} w_i x_i + b)$, thresholded to a binary flag. This is the smallest model that still exercises the same arithmetic as the production tree ensemble.

*   **Non-linearities:** the sigmoid is replaced by a low-degree polynomial approximation (degree 3) so the circuit stays arithmetic; this is the standard ZKML trick.
*   **Constraint count:** $\approx 16$ multiplications for the dot product, $\approx 3$ for the polynomial sigmoid, plus range checks on each 8-bit input/weight. At ~$2$ constraints per 8-bit range check this lands at **~$2\text{–}3\times10^3$ constraints** for the logistic scorer.
*   **Tree ensemble:** a depth-4, 8-tree gradient-boosted ensemble with the same 16 features is **~$10^4\text{–}10^5$ constraints** depending on how comparisons are encoded (comparisons are the expensive part).
*   **Proving time (off-chain, published benchmarks):** Groth16 at $10^4$ constraints is ~1–3 s and ~$10^5$ constraints ~10–30 s on a single core; STARK provers are ~10–100× slower for the same circuit but need no trusted setup.

### 3. On-chain cost model vs. transaction limits

Soroban transaction limits (protocol 21): **~100M CPU instructions**, **~40 MB memory**, and a **~100 KB** transaction size envelope. Using the existing Bulletproof verifier as the calibration point (an 8-bit range proof is ~800 bytes and verifies in the low millions of instructions), we extrapolate:

| Verifier | Proof size | Est. CPU instructions | Fits 100M budget? |
| --- | --- | --- | --- |
| Bulletproof, 8-bit (shipped) | ~0.8 KB | ~$10^6$ | Yes |
| STARK, $10^3$ constraints | ~50–200 KB | ~$10^7\text{–}10^8$ | Borderline; proof size may exceed tx envelope |
| STARK, $10^4$ constraints | ~0.5–2 MB | $>10^8$ | **No** |
| Groth16, $10^3$ constraints (if pairings enabled) | ~0.2 KB | ~$10^6\text{–}10^7$ | Yes |
| Groth16, $10^4$ constraints (if pairings enabled) | ~0.2 KB | ~$10^7$ | Yes |

**Finding:** hash-only STARKs blow the transaction size envelope well before the CPU budget for anything beyond a toy circuit. Pairing-based SNARKs (Groth16/PLONK) are the only family that keeps both proof size and verifier cost inside Soroban limits for a realistic small model — **conditional on the pairing host functions being available**. A Bulletproof-style verifier over Ed25519 is a viable fallback but scales linearly with circuit size and is only practical for very small models.

### 4. What would be proven, and how it connects to existing commitments

A ZKML inference proof would attest to the tuple:

*   **Input commitment** $C_{in}$ — a Pedersen/hash commitment to the feature vector $x$, bound to the **data-window commitment** already used for score freshness (same window id, same commitment scheme), so the proof is tied to a specific observation window.
*   **Model commitment** $C_{model}$ — a hash of the model weights, anchored to the **model card** (the model-card hash already published off-chain), so the proof is tied to a specific, auditable model version.
*   **Output** $y$ — the score, revealed or itself committed, with a range proof (reusing `zk_range_proof.rs`) that $y$ lies in the valid score domain.

The statement proven is: *"there exists $x$ opening $C_{in}$ and weights $w$ opening $C_{model}$ such that $y = f_w(x)$ and $y$ is in range."* This composes cleanly with the existing range proof: the ZKML proof establishes provenance, the range proof establishes the threshold predicate, and both reference the same model-card and data-window anchors.

### 5. Recommendation

**Proceed, conditionally.** ZKML inference verification is feasible on Soroban for a small model **only** via a pairing-based SNARK (Groth16/PLONK) and **only if** the `bn254`/`bls12-381` host functions are enabled on the target network. If pairings are unavailable, the recommendation is to **defer** — hash-only STARKs do not fit the transaction envelope, and Ed25519 Bulletproof-style verifiers do not scale past toy circuits.

### 6. Prerequisite issues (if proceeding)

1.  Confirm `bn254`/`bls12-381` host-function availability and cost on the target Soroban network; publish measured instruction counts.
2.  Define the model-card anchoring format and the data-window commitment scheme as a shared, versioned type.
3.  Specify the ZKML statement (input/model/output commitments) and its ABI, including how it composes with `verify_score_range_proof`.
4.  Build an off-chain prover for the toy logistic scorer and benchmark proof generation.
5.  Prototype the on-chain Groth16 verifier and measure against the 100M-instruction / 100 KB limits.
6.  Decide on a trusted-setup / ceremony process if Groth16 is chosen.
