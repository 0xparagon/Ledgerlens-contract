# Differential-testing findings

Each disagreement between the reference model (written from the docs) and the contract is
classified as a **code defect**, a **documentation defect**, or an **intentional deviation**, and
gets a fix or a documented rationale. Found with `run_diff.py` (issue #1240).

| ID | Area | Observed | Class | Resolution |
|---|---|---|---|---|
| D1 | Gate score comparison | Docs: pass iff `score >= threshold`. Contract: pass iff `score < threshold`. Repro: score 0, threshold 40 → contract `true`, docs `false`. | Documentation defect | The contract is correct: higher scores are more suspicious and the gate fails closed. Fixed the formula and all four truth tables in `docs/score-math.md` § Confidence-Floor Semantics. |
| D2 | Aggregate decay scope | Docs implied decay only applies past the staleness window. Contract decays every pair by its age. Repro: pairs {0, 100}, decay 1/1e6 → docs 50, contract 100. | Documentation defect | Fixed `docs/score-math.md` § Exponential Decay: decay applies at every age when the numerator is non-zero. |
| D3 | Aggregate with all decayed weights zero | Docs did not say what happens; the model assumed `0`. Contract returns `Err(ScoreNotFound)`. | Documentation gap | Documented in `docs/score-math.md` § Off-Chain Simulation (`weight_sum == 0`). |
| D4 | `get_effective_score` decay | Docs: decay only when `age > staleness_window`. Contract decays at any age. Repro: score 65, decay 2/1e7, age 3601 → docs 65, contract 64. | Documentation defect | Fixed `docs/score-math.md` § Staleness Filtering. |
| D5 | Hysteresis band | The glossary described the band relative to "the gate threshold". Band entry actually uses the contract-level `risk_threshold` (default 75), independent of the query threshold. Repro: score 99, query threshold 100 → contract `false`. | Documentation defect | Clarified `docs/glossary.md` § Hysteresis (risk band). |
| D6 | Decay approximation | The model and contract agree, but both implement a 4-term Taylor series. It is non-monotonic: minimum ≈0.27 at x≈1.6, then it rebounds and clamps to `SCALE` (no decay) near x≈3.3, well before the documented `x >= 5` cutoff. Very old scores regain full weight. | Specification + code defect | **Open.** Changing on-chain math alters stored-score semantics and needs a design note under the interface-versioning policy. Suggested fix: return 0 once x passes the series minimum (≈1.59·SCALE), or use more terms with a monotonic cutoff. Pinned by `test_known_defect_d6_decay_rebounds` so the fix must update the test deliberately. |

After D1–D5 the model follows the corrected docs, and 100,000 generated sequences run with zero
disagreements.
