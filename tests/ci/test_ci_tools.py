"""Tests for tools/ci/*.py (issues #1223, #1224, #1225).

Run with: python3 -m unittest discover -s tests/ci
"""

import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "ci"))

import coverage_gate  # noqa: E402
import job_durations  # noqa: E402
import nextest_report  # noqa: E402
import partition_tests  # noqa: E402


def llvm_cov(files):
    return {"data": [{"files": [
        {"filename": str(ROOT / rel), "summary": {
            "lines": {"count": 100, "covered": line},
            "branches": {"count": 0, "covered": 0},
        }} for rel, line in files.items()
    ]}]}


class CoverageGateTest(unittest.TestCase):
    def run_gate(self, files, baseline=None, changed=None):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            (d / "r.json").write_text(json.dumps(llvm_cov(files)))
            argv = [str(d / "r.json"), "--config", str(ROOT / "coverage-thresholds.toml"), "--root", str(ROOT)]
            if baseline is not None:
                (d / "b.json").write_text(json.dumps(llvm_cov(baseline)))
                (d / "c.txt").write_text("\n".join(changed))
                argv += ["--baseline", str(d / "b.json"), "--changed", str(d / "c.txt")]
            return coverage_gate.main(argv)

    def test_passes_when_thresholds_met(self):
        self.assertEqual(self.run_gate({"contracts/ledgerlens-score/src/storage.rs": 90}), 0)

    def test_seeded_drop_in_critical_module_fails(self):
        self.assertEqual(self.run_gate({"contracts/ledgerlens-score/src/storage.rs": 60}), 1)

    def test_peripheral_threshold_is_lower(self):
        self.assertEqual(self.run_gate({"tools/replay/src/main.rs": 45}), 0)

    def test_test_modules_are_excluded(self):
        self.assertEqual(self.run_gate({"contracts/ledgerlens-score/src/test_epoch.rs": 0}), 0)

    def test_changed_file_may_not_decrease(self):
        rel = "tools/replay/src/main.rs"
        self.assertEqual(self.run_gate({rel: 70}, baseline={rel: 75}, changed=[rel]), 1)
        self.assertEqual(self.run_gate({rel: 75}, baseline={rel: 75}, changed=[rel]), 0)


JUNIT = """<?xml version="1.0"?>
<testsuites><testsuite name="ledgerlens-score">
  <testcase classname="ledgerlens-score" name="test::ok" time="0.5"/>
  <testcase classname="ledgerlens-score" name="test::slow" time="42.0"/>
  <testcase classname="ledgerlens-score" name="test::network_retry" time="1.0">
    <flakyFailure message="timeout"/>
  </testcase>
</testsuite></testsuites>
"""


class NextestReportTest(unittest.TestCase):
    def test_retry_pass_is_flaky_not_green(self):
        with tempfile.TemporaryDirectory() as d:
            Path(d, "junit.xml").write_text(JUNIT)
            out = Path(d, "out.json")
            self.assertEqual(nextest_report.main([d, "--json", str(out), "--fail-on-flaky"]), 1)
            data = json.loads(out.read_text())
            self.assertEqual(data["flaky"], ["ledgerlens-score::test::network_retry"])
            self.assertEqual(data["slow"][0]["test"], "ledgerlens-score::test::slow")


class PartitionTest(unittest.TestCase):
    def test_every_test_in_exactly_one_balanced_shard(self):
        tests = [("bin", f"t{i}") for i in range(10)]
        durations = {("bin", f"t{i}"): 0.1 for i in range(1, 10)}
        durations[("bin", "t0")] = 10.0
        buckets, loads = partition_tests.partition(tests, durations, 3)
        self.assertEqual(sorted(t for b in buckets for t in b), sorted(tests))
        self.assertEqual(buckets[0], [("bin", "t0")])  # the long test gets its own shard
        self.assertIn("binary_id(=bin)", partition_tests.to_filterset(buckets[1]))


class JobDurationsTest(unittest.TestCase):
    def test_budget_exceeded_warns(self):
        jobs = [
            {"name": "Unit tests", "conclusion": "success",
             "started_at": "2026-01-01T00:00:00Z", "completed_at": "2026-01-01T00:30:00Z"},
            {"name": "Clippy", "conclusion": "success",
             "started_at": "2026-01-01T00:00:00Z", "completed_at": "2026-01-01T00:05:00Z"},
        ]
        record = job_durations.summarise_run({"id": 1, "head_sha": "abc", "created_at": "x"}, jobs)
        self.assertEqual(record["wall_clock_seconds"], 1800)
        warnings = job_durations.budget_warnings(record, {"required_checks_seconds": 1200, "jobs": {"Clippy": 600}})
        self.assertEqual(len(warnings), 1)


if __name__ == "__main__":
    unittest.main()
