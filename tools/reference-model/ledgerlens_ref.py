"""Independent reference model of LedgerLens score semantics (issue #1240).

Written from docs/score-math.md and docs/interface-spec.md only, never from
contracts/ledgerlens-score/src. Where the docs are silent, the assumption made
is marked ASSUMPTION. Every disagreement found by differential testing is
recorded in DISAGREEMENTS.md; once resolved, the model follows the corrected
documentation.

Every operation returns the same observable shape as driver/src/main.rs:
"ok", {"err": code}, a value, or None (for time advances).
"""

from __future__ import annotations

SCALE = 1_000_000
START_TIME = 1_700_000_000
DEFAULT_RISK_THRESHOLD = 75  # configuration-safe-defaults.md
DEFAULT_HYSTERESIS_MARGIN = 0

# Error discriminants from docs/errors.md.
INVALID_SCORE = 4
INVALID_CONFIDENCE = 5
SCORE_NOT_FOUND = 6


def decay_factor(age_secs: int, lambda_num: int, lambda_den: int) -> int:
    """score-math.md § Exponential Decay, Off-Chain Reference (verbatim)."""
    if lambda_num == 0:
        return SCALE
    x = (lambda_num * age_secs * SCALE) // lambda_den
    if x >= 5 * SCALE:
        return 0
    s = SCALE
    r = s - x + (x * x) // (2 * s) - (x * x * x) // (6 * s * s) + (x ** 4) // (24 * s ** 3)
    return max(0, min(r, s))


def aggregate(pairs_and_scores, pair_weights, decay_factors=None) -> int:
    """score-math.md § Weighted Average, Off-Chain Simulation (verbatim)."""
    decay_factors = decay_factors or {}
    weighted_sum = weight_sum = 0
    for pair, score in pairs_and_scores:
        w = (pair_weights.get(pair, 1) * decay_factors.get(pair, SCALE)) // SCALE
        weighted_sum += w * score
        weight_sum += w
    if weight_sum == 0:
        raise ValueError("All weights are zero")
    return weighted_sum // weight_sum


def gate_passes(score: int, threshold: int, conf: int, query_conf: int, floor: int) -> bool:
    """score-math.md § Confidence-Floor Semantics, truth tables 1-4 (D1)."""
    return score < threshold and conf >= query_conf and conf >= floor


class Model:
    def __init__(self) -> None:
        self.now = START_TIME
        self.scores: dict[tuple[int, str], tuple[int, int, int]] = {}
        self.weights: dict[str, int] = {}
        self.decay = (0, 1)
        self.floor = 0
        self.in_band: set[tuple[int, str]] = set()

    def _decay(self, ts: int) -> int:
        """score-math.md § Exponential Decay: applies at every age (D2/D4)."""
        return decay_factor(self.now - ts, *self.decay)

    def advance(self, secs):
        self.now += secs

    def submit(self, w, p, score, conf):
        if score > 100:
            return {"err": INVALID_SCORE}
        if conf > 100:
            return {"err": INVALID_CONFIDENCE}
        self.scores[(w, p)] = (score, conf, self.now)
        # glossary.md § Hysteresis (risk band) (D5)
        if score >= DEFAULT_RISK_THRESHOLD:
            self.in_band.add((w, p))
        elif score < DEFAULT_RISK_THRESHOLD - DEFAULT_HYSTERESIS_MARGIN:
            self.in_band.discard((w, p))
        return "ok"

    def weight(self, p, weight):
        self.weights[p] = weight  # ASSUMPTION: docs define no upper bound.
        return "ok"

    def set_decay(self, num, den):
        self.decay = (num, den)  # ASSUMPTION: den >= 1 (generator never sends 0).
        return "ok"

    def set_floor(self, v):
        if v > 100:  # ASSUMPTION: confidence domain is 0-100.
            return {"err": INVALID_CONFIDENCE}
        self.floor = v
        return "ok"

    def get(self, w, p):
        if (w, p) not in self.scores:
            return {"err": SCORE_NOT_FOUND}
        s, c, _ = self.scores[(w, p)]
        return [s, c]

    def eff(self, w, p):
        """score-math.md § Staleness Filtering in get_effective_score."""
        if (w, p) not in self.scores:
            return {"err": SCORE_NOT_FOUND}
        s, _, ts = self.scores[(w, p)]
        return s * self._decay(ts) // SCALE

    def agg(self, w):
        rows = sorted((p, v) for (ww, p), v in self.scores.items() if ww == w)
        if not rows:
            return {"err": SCORE_NOT_FOUND}
        decays = {p: self._decay(ts) for p, (_, _, ts) in rows}
        try:
            a = aggregate([(p, s) for p, (s, _, _) in rows], self.weights, decays)
        except ValueError:
            return {"err": SCORE_NOT_FOUND}  # all decayed weights zero (D3)
        return [a, len(rows), max(s for _, (s, _, _) in rows)]

    def gate(self, w, p, t, q):
        if (w, p) not in self.scores or (w, p) in self.in_band:
            return False  # fail-closed, interface-spec.md; hysteresis band
        s, c, _ = self.scores[(w, p)]
        return gate_passes(s, t, c, q, self.floor)

    def apply(self, op: dict):
        kind = op["op"]
        if kind == "advance":
            return self.advance(op["secs"])
        if kind == "submit":
            return self.submit(op["w"], op["p"], op["score"], op["conf"])
        if kind == "weight":
            return self.weight(op["p"], op["weight"])
        if kind == "decay":
            return self.set_decay(op["num"], op["den"])
        if kind == "floor":
            return self.set_floor(op["v"])
        if kind in ("get", "eff"):
            return getattr(self, kind)(op["w"], op["p"])
        if kind == "agg":
            return self.agg(op["w"])
        if kind == "gate":
            return self.gate(op["w"], op["p"], op["t"], op["q"])
        raise ValueError(f"unknown op {kind}")


def run(ops: list[dict]) -> list:
    m = Model()
    return [m.apply(op) for op in ops]
