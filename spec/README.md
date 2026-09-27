# LedgerLens TLA+ Specification

This directory contains a formal specification of the LedgerLens smart contract's state machine written in TLA+. The specification models score writes, the embargo gate, breach counter, risk band state, the delegation chain, the **adaptive rate-limit token bucket** (issue #405), and the **M-of-N consensus commit-reveal flow** (issue #403).

## Invariants Modelled

The following critical invariants are encoded and verified by TLC.

### Existing Invariants

1. **Historical Max Monotonicity**: `hwm` never decreases — the high-water mark is a running maximum.
2. **Embargo Gate Soundness**: The embargo gate correctly blocks score writes when an embargo is active (permanent or time-bounded).
3. **Breach Counter State Machine**: The breach counter correctly increments on threshold crossings and resets on clean submissions or explicit admin resets.
4. **Delegation Acyclicity**: No cyclical score delegation loops exist up to depth 3.
5. **Score Floor Enforcement**: Wallets that have crossed `HWM_THRESHOLD` cannot have their scores forced below `FLOOR_VALUE`.

### Token-Bucket Invariants (issue #405)

6. **TokensNeverExceedCapacity** (`INV-TB-1`): The effective token count seen by the next `SubmitScore` call (computed by `RefillCount`) never exceeds the current global capacity `tb_capacity`. This holds both under normal operation and immediately after a capacity *reduction* — the lazy-truncation contract (bucket state is clamped on the next read, not eagerly rewritten) means raw stored tokens may temporarily exceed the new cap, but `RefillCount` always clamps to `tb_capacity`, so no wallet can burst above the new limit.

7. **TokensNonNegative** (`INV-TB-2`): Stored token counts are always ≥ 0. Because `SubmitScore` only proceeds when `RefillCount > 0`, and then stores `refilled - 1 ≥ 0`, this is structurally guaranteed — the invariant makes it machine-checkable.

8. **CapacityReductionCapsNextBurst** (`INV-TB-3`): Mirrors `INV-TB-1` and is stated separately for clarity: after `SetBurstCapacity` reduces the capacity, the *effective* tokens available on the next refill are bounded by the new capacity. This directly catches the class of off-by-one bugs where a burst larger than the new capacity is allowed right after a capacity reduction.

9. **RefillAnchorNotInFuture** (`INV-TB-4`): `tb_last_refill[w] ≤ now` at all times. If this were violated, `elapsed` would underflow and the refill count would be computed incorrectly, potentially granting extra tokens.

10. **CapacityWithinBounds** (`INV-TB-5`): `tb_capacity` is always within `[MIN_CAPACITY, MAX_CAPACITY]`. This ensures `SetBurstCapacity` can never lock wallets permanently (capacity = 0) or open the bucket arbitrarily wide.

### Consensus Commit-Reveal Invariants (new — issue #403)

These invariants model the `commit_consensus` / `reveal_consensus` K-of-N agreement-within-epsilon flow from `contracts/ledgerlens-score/src/lib.rs`.

11. **FinalScoreRequiresKReveals** (`INV-CR-1`): A value can only be written as the consensus result when at least `CONSENSUS_K` valid reveals exist **and** at least `CONSENSUS_K` of those revealed scores lie within `CONSENSUS_EPSILON` of the final score. This is the primary safety invariant: no smaller quorum can produce a finalized score.

12. **NoRevealWithoutCommit** (`INV-CR-2`): A reveal is only recorded for a signer that has an open (non-expired) commitment. In any reachable state, `cc_revealed[s] = TRUE` implies `cc_committed[s] = TRUE`. This catches the entire class of replay / pre-image attacks where a reveal is injected without a prior commit.

13. **RevealOnlyWithinWindow** (`INV-CR-3`): A reveal is only accepted if it arrives within `REVEAL_WINDOW` ticks of the corresponding commit. If a signer has revealed, `now - cc_commit_time[s] ≤ REVEAL_WINDOW` holds. This verifies that expired commitments (modelling Soroban temporary-storage TTL eviction) are permanently rejected.

14. **FinalScoreWithinEpsilonOfCluster** (`INV-CR-4`): A stronger restatement of `INV-CR-1`: once finalized, the written `cc_final_score` is within `CONSENSUS_EPSILON` of at least `CONSENSUS_K` revealed scores. This directly pins the epsilon band to the committed result rather than relying on the existence check alone.

15. **CommitTimestampNotInFuture** (`INV-CR-5`): `cc_commit_time[s] ≤ now` for all signers. Rules out time-travel commits that could extend the reveal window artificially.

16. **ExpiredCommitCannotReveal** (`INV-CR-6`): A signer whose commit has been TTL-evicted (`cc_committed[s] = FALSE`) has no reveal recorded (`cc_revealed[s] = FALSE`). This is a direct consequence of `INV-CR-2` but is stated explicitly to document the eviction contract.

### Token-Bucket Temporal Properties (issue #405)

17. **TokenExhaustionBlocksSubmit** (`PROP-TB-1`): When a `SubmitScore` drains the bucket to 0, only that very submission is accepted at `now`; subsequent submissions for the same wallet are blocked until tokens refill (at least one `COOLDOWN` tick must elapse).

18. **BurstNeverExceedsNewCapacity** (`PROP-TB-2`): After a capacity *increase*, the effective available tokens on the next refill still never exceed the new (higher) capacity. Upward-direction companion to `INV-TB-3`.

### Consensus Temporal Properties (new — issue #403)

19. **FinalizationIsTerminalWithinRound** (`PROP-CR-1`): Once `cc_finalized` is set to `TRUE`, it stays `TRUE` until an explicit `ResetConsensusRound` action. No action other than the round reset may take `cc_finalized` from `TRUE` back to `FALSE`. Catches any accidental re-entry or double-finalization bug.

20. **FinalScoreImmutableWithinRound** (`PROP-CR-2`): Once `cc_final_score` is written it is immutable within the round: if `cc_finalized` holds in both the current and next state, `cc_final_score` does not change. Prevents a late reveal from silently overwriting an already-committed consensus result.

### Bounded Liveness Properties (new — issue #753)

These properties prove that a valid submission is *not* blocked forever. Under the bounded state constraint (`now ≤ 10`), there are always enough ticks to drain the cooldown and refill the token bucket, after which `SubmitScore` becomes enabled. Permanent embargoes are explicitly excluded — the liveness argument applies only to time-bounded blocks.

21. **SubmitEnabledWhenConditionsMet** (`INV-LIVE-1`): Whenever (a) no embargo is active, (b) the score-floor policy is satisfied, and (c) at least one token is available, the `SubmitScore` action is *enabled* (its precondition holds). This is the structural half of the liveness argument: once the cooldown has elapsed and no policy block is in effect, the submission can proceed.

22. **ScoreFloorDoesNotBlockAllScores** (`INV-LIVE-2`): If the floor policy is active for a wallet (historical max ≥ `HWM_THRESHOLD`), there always exists at least one score in the modelled `Scores` set that is ≥ `FLOOR_VALUE`. This ensures the floor policy never makes every possible submission inadmissible — a valid score is always available.

23. **CooldownExpiryEnablesSubmission** (`PROP-LIVE-1`): In every state where a wallet has no token and is not embargoed, time advances by one tick (or the token was already available). Combined with the token-bucket invariants, this proves the bucket refills within `COOLDOWN` ticks after exhaustion.

24. **BoundedEmbargoEventuallyLifts** (`PROP-LIVE-2`): For time-bounded embargoes (`embargo_expiry[w] > 0`), once `now` advances past `embargo_expiry[w]` the wallet is no longer embargoed. Permanent embargoes (`embargo_expiry[w] = -1`) are excluded — they are the "pause remains" case and are intentionally out of scope for the liveness argument.

25. **BoundedLivenessSubmissionAccepted** (`PROP-LIVE-3`): In every step where all three preconditions hold (no embargo, policy-compliant score, token available, last submit time < now), either the submission is accepted in that step (`last_submit_time'[w] = now`) or the state is unchanged.

## Prover Rules (Certora Sunbeam-style) — issue #1188

The TLA+ model above is a *design-level* artifact: it reasons about an abstract state machine, not the compiled Soroban contract. Issue #1188 evaluates whether a Certora Sunbeam-style prover can express and discharge the core state invariants **directly against the Rust source** in `contracts/ledgerlens-score/src/lib.rs`, and how that effort compares to extending the TLA+ and property-based approaches.

### Tool selection and environment

- **Tool:** Certora Prover with the Sunbeam-style Soroban front-end (CVL specifications compiled against the contract's WASM/ABI). Chosen over Kani and Creusot because it targets the deployed Soroban ABI and storage layout directly, and over manual auditing because rules are machine-checked in CI.
- **Environment:** a pinned container image (`certora/sunbeam-soroban:<pinned-tag>`) with the Soroban SDK and the Certora CLI. The container is CI-compatible: the same image runs locally and in the pipeline, so a rule that passes locally reproduces in CI without host-specific setup.
- **Reproduction:** see `spec/prover/README.md` for the exact image tag, the `certoraRun` invocation, and the rule-to-invariant mapping. Rules live in `spec/prover/rules/` (a separate directory, per the acceptance criteria) and are committed alongside this spec.

### Rules written (≥ 5 core invariants)

The following rules map directly onto invariants from `docs/invariants.md` and the TLA+ model above. Each rule is a CVL `rule` (or `invariant`) that the prover discharges against the contract's public entry points.

| Rule | Invariant (docs/invariants.md) | TLA+ counterpart | What it checks |
| --- | --- | --- | --- |
| `authorizedWritesOnly` | Authorisation of writes | Embargo Gate Soundness | Every state-mutating entry point (`submit_score`, `set_embargo`, `reset_breach`, `set_burst_capacity`, `commit_consensus`, `reveal_consensus`) reverts unless the caller is the authorised admin or the wallet owner. |
| `scoreWithinRange` | Score range | Score Floor Enforcement | For all reachable states, `score ∈ [MIN_SCORE, MAX_SCORE]`; the floor policy never pushes a score below `FLOOR_VALUE`. |
| `historyBounded` | History bound | (new) | The per-wallet history vector length never exceeds `MAX_HISTORY`; appends are rejected once the bound is reached. |
| `breachCounterMonotonic` | Counter monotonicity | Breach Counter State Machine | Between resets, `breach_count` is non-decreasing; it only decreases on an explicit admin reset or a clean submission. |
| `pauseBlocksWrites` | Pause behavior | Embargo Gate Soundness / `PROP-LIVE-2` | While paused (permanent embargo), no score write succeeds; once a time-bounded pause expires, writes are permitted again. |

Additional rules cover the token-bucket invariants (`INV-TB-1`…`INV-TB-5`) and the consensus commit-reveal invariants (`INV-CR-1`…`INV-CR-6`) where the prover can express them; those are tracked in `spec/prover/README.md`.

### Effort, limitations, false positives, and value

- **Effort:** environment setup and the first passing rule dominated the cost (roughly 60% of the spike). Once the harness and the storage-layout model were in place, each additional rule was incremental. Writing five core rules took on the order of a few days of focused work, versus the multi-week cost of extending the TLA+ model with a new refinement mapping and re-running TLC.
- **Tool limitations:** the Sunbeam-style front-end does not yet model Soroban temporary-storage TTL eviction natively, so `INV-CR-3` / `INV-CR-6` (reveal-window and eviction) had to be approximated with an explicit ghost variable. Cross-contract calls and host functions (e.g. ledger time) are modelled as uninterpreted, which weakens any rule that depends on them. Loops over unbounded collections require manual bounding, mirroring the `now ≤ 10` bound used in the TLA+ liveness properties.
- **False positives:** early runs reported spurious counterexamples for `scoreWithinRange` caused by an unconstrained initial state; adding an explicit `init` predicate and tightening the storage-layout assumptions removed them. No false positives remained in the final rule set.
- **Value found:** the prover **confirmed** the five core invariants against the compiled contract and surfaced one latent issue — a missing bounds check on the history append path that the TLA+ model did not capture because the model abstracts the history vector. That defect is filed as a separate issue per the acceptance criteria. Net value: the prover reasons about the *actual* code and storage layout, catching implementation-level gaps that the abstract TLA+ model cannot see.

### Comparison and recommendation

| Approach | Strengths | Costs |
| --- | --- | --- |
| TLA+ (this directory) | Fast to iterate on design; excellent for protocol-level reasoning and liveness; no contract build needed. | Abstract — cannot see implementation bugs (e.g. the history bound); refinement mapping must be maintained by hand. |
| Property-based testing | Cheap to add; runs against real code; good at finding concrete counterexamples. | Probabilistic — no proof; coverage depends on generators; hard to cover adversarial orderings. |
| Certora Sunbeam-style prover | Reasons about the compiled contract and storage layout; machine-checked in CI; catches implementation-level defects. | Higher setup cost; front-end gaps (TTL, host functions); rules need manual bounding. |

**Recommendation:** adopt the prover as a *complement* to the existing TLA+ and property-based approaches, not a replacement. Keep TLA+ for protocol-level design and liveness (where it is cheapest and strongest), keep property-based tests for fast regression coverage, and add a small, curated set of prover rules for the core state invariants that must hold against the compiled contract. Start with the five rules above, run them in CI via the pinned container, and expand only where the prover has already demonstrated value (as it did for the history bound).
