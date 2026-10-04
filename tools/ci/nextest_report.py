#!/usr/bin/env python3
"""Flaky and slow test report from nextest JUnit output (issue #1223).

A test that failed at least once and then passed on retry carries a
``<flakyFailure>`` element in nextest's JUnit output. Such tests are reported
as FLAKY, never as plain passes: the report emits a GitHub warning per flaky
test and, with ``--fail-on-flaky``, exits non-zero so the report job is red.

Outputs a Markdown summary (stdout, or appended to $GITHUB_STEP_SUMMARY) and a
JSON record suitable for trend tracking (``--json``).
"""

import argparse
import json
import os
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


def collect(junit_dir):
    cases = []
    for report in sorted(Path(junit_dir).rglob("*.xml")):
        root = ET.parse(report).getroot()
        for case in root.iter("testcase"):
            tags = {child.tag for child in case}
            cases.append({
                "binary": case.get("classname", ""),
                "name": case.get("name", ""),
                "time": float(case.get("time", "0") or 0),
                "failed": bool(tags & {"failure", "error"}),
                "flaky": "flakyFailure" in tags or "flakyError" in tags,
                "attempts": 1 + sum(1 for c in case if c.tag in ("flakyFailure", "flakyError", "rerunFailure", "rerunError")),
            })
    return cases


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("junit_dir")
    ap.add_argument("--slow-threshold", type=float, default=10.0, help="seconds")
    ap.add_argument("--top", type=int, default=20)
    ap.add_argument("--json", help="write machine-readable summary here")
    ap.add_argument("--fail-on-flaky", action="store_true")
    args = ap.parse_args(argv)

    cases = collect(args.junit_dir)
    flaky = sorted((c for c in cases if c["flaky"] and not c["failed"]), key=lambda c: (c["binary"], c["name"]))
    failed = [c for c in cases if c["failed"]]
    slow = sorted((c for c in cases if c["time"] >= args.slow_threshold), key=lambda c: -c["time"])[: args.top]
    total_time = sum(c["time"] for c in cases)

    lines = [
        "## Test report",
        "",
        f"- Tests: **{len(cases)}**, failed: **{len(failed)}**, flaky: **{len(flaky)}**",
        f"- Summed test time: **{total_time:.1f}s**",
        "",
    ]
    if flaky:
        lines += ["### Flaky (passed only after retry)", "", "| Binary | Test | Attempts |", "|---|---|---|"]
        lines += [f"| `{c['binary']}` | `{c['name']}` | {c['attempts']} |" for c in flaky]
        lines.append("")
    if slow:
        lines += [f"### Slow (>= {args.slow_threshold:.0f}s)", "", "| Binary | Test | Seconds |", "|---|---|---|"]
        lines += [f"| `{c['binary']}` | `{c['name']}` | {c['time']:.2f} |" for c in slow]
        lines.append("")
    markdown = "\n".join(lines)

    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with open(summary_path, "a") as fh:
            fh.write(markdown + "\n")
    print(markdown)

    for c in flaky:
        print(f"::warning title=Flaky test::{c['binary']} {c['name']} passed only after {c['attempts']} attempts")

    if args.json:
        Path(args.json).write_text(json.dumps({
            "tests": len(cases),
            "failed": len(failed),
            "flaky": [f"{c['binary']}::{c['name']}" for c in flaky],
            "slow": [{"test": f"{c['binary']}::{c['name']}", "seconds": round(c["time"], 3)} for c in slow],
            "total_seconds": round(total_time, 3),
        }, indent=2, sort_keys=True) + "\n")

    if args.fail_on_flaky and flaky:
        print(f"{len(flaky)} flaky test(s) detected; failing the report so the run is not green.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
