"""Tests for the reference model, derived from docs/score-math.md examples and
truth tables. Run: python3 -m unittest discover tools/reference-model"""

import re
import unittest
from pathlib import Path

import ledgerlens_ref as ref

DOC = Path(__file__).resolve().parents[2] / "docs" / "score-math.md"


class Decay(unittest.TestCase):
    def test_zero_rate_is_identity(self):
        self.assertEqual(ref.decay_factor(10**9, 0, 1), ref.SCALE)

    def test_age_zero_is_identity(self):
        self.assertEqual(ref.decay_factor(0, 1, 1), ref.SCALE)

    def test_large_x_saturates_to_zero(self):
        self.assertEqual(ref.decay_factor(5, 1, 1), 0)

    def test_monotone_below_taylor_minimum(self):
        # x = age * SCALE / 1_000_000 here, so age == x_scaled; minimum is at x ~= 1.596.
        vals = [ref.decay_factor(t, 1, 1_000_000) for t in range(0, 1_590_000, 10_000)]
        self.assertEqual(vals, sorted(vals, reverse=True))

    def test_known_defect_d6_decay_rebounds(self):
        """DISAGREEMENTS.md D6: the documented 4-term series rebounds past x ~= 1.6
        and clamps back to SCALE (no decay) before the x >= 5 cutoff. Code matches
        the docs; this test pins the behaviour until the spec is fixed."""
        at_min = ref.decay_factor(1_600_000, 1, 1_000_000)
        self.assertLess(at_min, 300_000)
        self.assertGreater(ref.decay_factor(3_000_000, 1, 1_000_000), at_min)
        self.assertEqual(ref.decay_factor(4_000_000, 1, 1_000_000), ref.SCALE)


class Aggregate(unittest.TestCase):
    def test_equal_weights(self):
        self.assertEqual(ref.aggregate([("A", 20), ("B", 81)], {}), 50)  # truncates

    def test_zero_weight_excludes_pair(self):
        self.assertEqual(ref.aggregate([("A", 20), ("B", 80)], {"A": 0}), 80)

    def test_all_zero_raises(self):
        with self.assertRaises(ValueError):
            ref.aggregate([("A", 20)], {"A": 0})

    def test_model_all_zero_weights_is_score_not_found(self):
        m = ref.Model()
        m.submit(0, "A", 10, 90)
        m.weight("A", 0)
        self.assertEqual(m.agg(0), {"err": ref.SCORE_NOT_FOUND})


class GateTruthTables(unittest.TestCase):
    """Every row of the four truth tables in score-math.md must hold."""

    def test_doc_tables(self):
        section = DOC.read_text().split("## Confidence-Floor Semantics", 1)[1].split("### Configuration Notes", 1)[0]
        rows = re.findall(r"^\|\s*(\d+)\s*\|\s*(\d+)\s*\|\s*(\d+)\s*\|\s*(\d+)\s*\|\s*(\d+)\s*\|\s*(PASS|FAIL)", section, re.M)
        self.assertGreaterEqual(len(rows), 20)
        for s, t, c, q, f, want in rows:
            with self.subTest(row=(s, t, c, q, f)):
                self.assertEqual(ref.gate_passes(int(s), int(t), int(c), int(q), int(f)), want == "PASS")


class ModelSemantics(unittest.TestCase):
    def test_validation(self):
        m = ref.Model()
        self.assertEqual(m.submit(0, "A", 101, 50), {"err": ref.INVALID_SCORE})
        self.assertEqual(m.submit(0, "A", 50, 101), {"err": ref.INVALID_CONFIDENCE})
        self.assertEqual(m.get(0, "A"), {"err": ref.SCORE_NOT_FOUND})

    def test_missing_score_fails_closed(self):
        self.assertFalse(ref.Model().gate(0, "A", 100, 0))

    def test_hysteresis_band(self):
        m = ref.Model()
        m.submit(0, "A", 99, 100)
        self.assertFalse(m.gate(0, "A", 100, 0))  # in band despite 99 < 100
        m.submit(0, "A", 10, 100)
        self.assertTrue(m.gate(0, "A", 100, 0))  # left band

    def test_decay_applies_at_any_age(self):
        m = ref.Model()
        m.submit(0, "A", 65, 50)
        m.set_decay(2, 10_000_000)
        m.advance(3_601)
        self.assertEqual(m.eff(0, "A"), 64)


if __name__ == "__main__":
    unittest.main()
