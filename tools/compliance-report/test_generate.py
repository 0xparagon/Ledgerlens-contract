"""Golden-file and determinism tests for the compliance report (issue #1222).

Run with: python3 -m unittest discover -s tools/compliance-report -p 'test_*.py'
Set UPDATE_GOLDEN=1 to regenerate testdata/golden/ after an intentional change.
"""

import json
import os
import random
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import generate  # noqa: E402

CORPUS = HERE / "testdata" / "corpus.jsonl"
GOLDEN = HERE / "testdata" / "golden"
ARGS = ["--ledger-from", "1000", "--ledger-to", "3000"]


def run(corpus, out):
    generate.main([str(corpus), "--out", str(out), *ARGS])
    return {name: (out / name).read_bytes() for name in ("report.json", "report.html")}


class GoldenTest(unittest.TestCase):
    def test_matches_golden_files(self):
        with tempfile.TemporaryDirectory() as d:
            produced = run(CORPUS, Path(d))
            if os.environ.get("UPDATE_GOLDEN"):
                GOLDEN.mkdir(parents=True, exist_ok=True)
                for name, data in produced.items():
                    (GOLDEN / name).write_bytes(data)
            for name, data in produced.items():
                self.assertEqual(data, (GOLDEN / name).read_bytes(), f"{name} differs from golden file")

    def test_hash_is_independent_of_corpus_order(self):
        lines = CORPUS.read_text().splitlines()
        random.Random(7).shuffle(lines)
        with tempfile.TemporaryDirectory() as d:
            shuffled = Path(d) / "corpus.jsonl"
            shuffled.write_text("\n".join(lines) + "\n")
            produced = run(shuffled, Path(d) / "out")
        self.assertEqual(produced["report.json"], (GOLDEN / "report.json").read_bytes())

    def test_content_hash_covers_body(self):
        report = json.loads((GOLDEN / "report.json").read_text())
        claimed = report.pop("content_hash")
        self.assertEqual(claimed, generate.hashlib.sha256(generate.canonical(report).encode()).hexdigest())

    def test_no_wallet_identifiers_leak(self):
        for name in ("report.json", "report.html"):
            self.assertNotIn(b"GWALLET", (GOLDEN / name).read_bytes())

    def test_small_buckets_suppressed(self):
        events = [{"type": "score", "ledger": 1, "contract_id": "C", "score": s} for s in [5] * 6 + [95]]
        dist = generate.score_section(events, 10, 5)["C"]
        hidden = [b["range"] for b in dist["buckets"] if b["suppressed"]]
        # The lone 95 is hidden, and complementary suppression hides the 0-9 bucket too.
        self.assertEqual(hidden, [[0, 9], [90, 100]])


if __name__ == "__main__":
    unittest.main()
