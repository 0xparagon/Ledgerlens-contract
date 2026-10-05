#!/usr/bin/env python3
"""Duration-balanced test partitioning for cargo-nextest (issue #1223).

Reads the current test list (``cargo nextest list --message-format json``)
and, optionally, JUnit reports from previous runs, then assigns every test to
one of N shards with a longest-processing-time-first greedy algorithm. Prints
the nextest filterset expression selecting the tests of the requested shard.

Tests without historical data are weighted with the median known duration so
new tests are spread across shards instead of piling onto one.

Usage:
    partition_tests.py --list tests.json --shards 4 --index 1 \
        [--junit-dir previous-reports/]
"""

import argparse
import json
import statistics
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

DEFAULT_WEIGHT = 0.05  # seconds, used when there is no history at all


def load_tests(path):
    data = json.loads(Path(path).read_text())
    tests = []
    for binary_id, suite in sorted(data.get("rust-suites", {}).items()):
        for name, case in sorted(suite.get("testcases", {}).items()):
            if case.get("ignored") or case.get("filter-match", {}).get("status", "matches") != "matches":
                continue
            tests.append((binary_id, name))
    return tests


def load_durations(junit_dir):
    durations = {}
    if not junit_dir:
        return durations
    for report in sorted(Path(junit_dir).rglob("*.xml")):
        try:
            root = ET.parse(report).getroot()
        except ET.ParseError:
            continue
        for case in root.iter("testcase"):
            key = (case.get("classname", ""), case.get("name", ""))
            try:
                t = float(case.get("time", "0"))
            except ValueError:
                continue
            # Keep the worst observed duration so shards are balanced pessimistically.
            durations[key] = max(t, durations.get(key, 0.0))
    return durations


def partition(tests, durations, shards):
    known = [durations[t] for t in tests if t in durations]
    fallback = statistics.median(known) if known else DEFAULT_WEIGHT
    weighted = sorted(
        ((durations.get(t, fallback), t) for t in tests),
        key=lambda wt: (-wt[0], wt[1]),
    )
    loads = [0.0] * shards
    buckets = [[] for _ in range(shards)]
    for weight, test in weighted:
        i = min(range(shards), key=lambda s: (loads[s], s))
        loads[i] += weight
        buckets[i].append(test)
    return buckets, loads


def to_filterset(tests):
    by_binary = {}
    for binary_id, name in sorted(tests):
        by_binary.setdefault(binary_id, []).append(name)
    if not by_binary:
        return "none()"
    parts = []
    for binary_id, names in by_binary.items():
        alts = " | ".join(f"test(={n})" for n in names)
        parts.append(f"(binary_id(={binary_id}) & ({alts}))")
    return " | ".join(parts)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--list", required=True, help="nextest list JSON")
    ap.add_argument("--junit-dir", help="directory with historical JUnit XML")
    ap.add_argument("--shards", type=int, required=True)
    ap.add_argument("--index", type=int, required=True, help="1-based shard index")
    ap.add_argument("--summary", action="store_true", help="print per-shard load to stderr")
    args = ap.parse_args(argv)
    if not 1 <= args.index <= args.shards:
        ap.error("--index must be within 1..--shards")

    tests = load_tests(args.list)
    buckets, loads = partition(tests, load_durations(args.junit_dir), args.shards)
    if args.summary:
        for i, (b, load) in enumerate(zip(buckets, loads), 1):
            print(f"shard {i}: {len(b)} tests, ~{load:.1f}s", file=sys.stderr)
    print(to_filterset(buckets[args.index - 1]))


if __name__ == "__main__":
    main()
