#!/usr/bin/env python3
"""Record CI job durations, publish a trend summary and enforce the
wall-clock budget (issue #1224).

Subcommands:
  collect  Fetch recent completed runs of a workflow via the GitHub REST API
           and append one JSON record per run to a JSON-lines dataset
           (runs already present are skipped, so the dataset is append-only).
  trend    Summarise the dataset: median wall-clock of required checks and of
           each job over a recent window versus the previous window, plus
           budget warnings for the most recent run.

Budgets live in .github/ci-budget.json. Requires GITHUB_TOKEN and
GITHUB_REPOSITORY for `collect`.
"""

import argparse
import json
import os
import statistics
import sys
import urllib.request
from datetime import datetime
from pathlib import Path

API = "https://api.github.com"


def _get(path):
    req = urllib.request.Request(API + path, headers={
        "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
        "Accept": "application/vnd.github+json",
    })
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.load(resp)


def _ts(value):
    return datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ")


def summarise_run(run, jobs):
    durations = {}
    starts, ends = [], []
    for job in jobs:
        if not job.get("started_at") or not job.get("completed_at") or job.get("conclusion") == "skipped":
            continue
        start, end = _ts(job["started_at"]), _ts(job["completed_at"])
        durations[job["name"]] = int((end - start).total_seconds())
        starts.append(start)
        ends.append(end)
    wall = int((max(ends) - min(starts)).total_seconds()) if starts else 0
    return {
        "run_id": run["id"],
        "head_sha": run.get("head_sha"),
        "created_at": run.get("created_at"),
        "wall_clock_seconds": wall,
        "jobs": dict(sorted(durations.items())),
    }


def budget_warnings(record, budget):
    warnings = []
    limit = budget.get("required_checks_seconds")
    if limit and record["wall_clock_seconds"] > limit:
        warnings.append(f"Required checks took {record['wall_clock_seconds']}s, budget is {limit}s")
    for name, job_limit in budget.get("jobs", {}).items():
        took = record["jobs"].get(name)
        if took is not None and took > job_limit:
            warnings.append(f"Job '{name}' took {took}s, budget is {job_limit}s")
    return warnings


def load_dataset(path):
    p = Path(path)
    if not p.exists():
        return []
    return [json.loads(line) for line in p.read_text().splitlines() if line.strip()]


def cmd_collect(args):
    repo = os.environ["GITHUB_REPOSITORY"]
    records = load_dataset(args.dataset)
    seen = {r["run_id"] for r in records}
    runs = _get(f"/repos/{repo}/actions/workflows/{args.workflow}/runs"
                f"?branch={args.branch}&status=completed&per_page={args.limit}")["workflow_runs"]
    added = 0
    for run in sorted(runs, key=lambda r: r["id"]):
        if run["id"] in seen:
            continue
        jobs = _get(f"/repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100")["jobs"]
        records.append(summarise_run(run, jobs))
        added += 1
    Path(args.dataset).parent.mkdir(parents=True, exist_ok=True)
    Path(args.dataset).write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in records))
    print(f"Recorded {added} new run(s); dataset has {len(records)}.")
    return 0


def _median(values):
    return statistics.median(values) if values else None


def cmd_trend(args):
    records = sorted(load_dataset(args.dataset), key=lambda r: r["run_id"])
    budget = json.loads(Path(args.budget).read_text())
    recent, previous = records[-args.window:], records[-2 * args.window:-args.window]
    lines = ["## CI duration trend", "",
             f"Window: last {len(recent)} run(s) vs previous {len(previous)} run(s) on main.", "",
             "| Scope | Median now | Median before | Budget |", "|---|---|---|---|"]

    def row(label, now, before, limit):
        f = lambda v: "-" if v is None else f"{v / 60:.1f} min"
        lines.append(f"| {label} | {f(now)} | {f(before)} | {f(limit)} |")

    row("**Required checks (wall clock)**",
        _median([r["wall_clock_seconds"] for r in recent]),
        _median([r["wall_clock_seconds"] for r in previous]),
        budget.get("required_checks_seconds"))
    for name in sorted({j for r in recent for j in r["jobs"]}):
        row(name, _median([r["jobs"][name] for r in recent if name in r["jobs"]]),
            _median([r["jobs"][name] for r in previous if name in r["jobs"]]),
            budget.get("jobs", {}).get(name))

    warnings = budget_warnings(recent[-1], budget) if recent else []
    if warnings:
        lines += ["", "### Budget exceeded (latest run)", ""] + [f"- {w}" for w in warnings]
    markdown = "\n".join(lines) + "\n"
    print(markdown)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as fh:
            fh.write(markdown)
    for w in warnings:
        print(f"::warning title=CI wall-clock budget::{w}")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("collect")
    c.add_argument("--dataset", required=True)
    c.add_argument("--workflow", default="ci.yml")
    c.add_argument("--branch", default="main")
    c.add_argument("--limit", type=int, default=30)
    t = sub.add_parser("trend")
    t.add_argument("--dataset", required=True)
    t.add_argument("--budget", default=".github/ci-budget.json")
    t.add_argument("--window", type=int, default=10)
    args = ap.parse_args(argv)
    return cmd_collect(args) if args.cmd == "collect" else cmd_trend(args)


if __name__ == "__main__":
    sys.exit(main())
