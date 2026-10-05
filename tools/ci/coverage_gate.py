#!/usr/bin/env python3
"""Per-module coverage gate over a `cargo llvm-cov --json` export (issue #1225).

Checks, in order:
  1. every module listed in coverage-thresholds.toml meets its line (and,
     when the report has branch data, branch) threshold;
  2. every other counted file meets the [default] threshold;
  3. "no decrease": files changed in the PR (``--changed``) must not lose
     line coverage relative to the ``--baseline`` report from main.

Writes a Markdown per-module table (stdout, $GITHUB_STEP_SUMMARY and
``--markdown``) and exits 1 on any violation.
"""

import argparse
import json
import os
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # Python < 3.11
    import tomli as tomllib


def pct(summary, kind):
    s = summary.get(kind) or {}
    if not s.get("count"):
        return None
    return 100.0 * s.get("covered", 0) / s["count"]


def load_report(path, root, excludes):
    data = json.loads(Path(path).read_text())
    files = {}
    for export in data.get("data", []):
        for f in export.get("files", []):
            name = f["filename"]
            try:
                rel = str(Path(name).resolve().relative_to(root))
            except ValueError:
                continue  # dependency sources outside the workspace
            if any(ex in rel if ex.startswith("/") else rel.startswith(ex) for ex in excludes):
                continue
            files[rel] = {"line": pct(f["summary"], "lines"), "branch": pct(f["summary"], "branches")}
    return files


def rule_for(rel, config):
    best = None
    for prefix, rule in config.get("modules", {}).items():
        if rel.startswith(prefix) and (best is None or len(prefix) > len(best[0])):
            best = (prefix, rule)
    return best or ("(default)", config.get("default", {}))


def fmt(v):
    return "n/a" if v is None else f"{v:.1f}%"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("report", help="cargo llvm-cov --json output")
    ap.add_argument("--config", default="coverage-thresholds.toml")
    ap.add_argument("--root", default=".")
    ap.add_argument("--baseline", help="baseline report from main for the no-decrease rule")
    ap.add_argument("--changed", help="file with repo-relative paths changed in the PR, one per line")
    ap.add_argument("--markdown", help="also write the Markdown summary here")
    args = ap.parse_args(argv)

    root = Path(args.root).resolve()
    config = tomllib.loads(Path(args.config).read_text())
    excludes = config.get("exclude", [])
    files = load_report(args.report, root, excludes)
    violations = []

    # Aggregate per rule (module) so the table reads per module, not per file.
    modules = {}
    for rel, cov in sorted(files.items()):
        prefix, rule = rule_for(rel, config)
        modules.setdefault(prefix, (rule, []))[1].append((rel, cov))

    rows = []
    for prefix, (rule, members) in sorted(modules.items()):
        lines = [c["line"] for _, c in members if c["line"] is not None]
        branches = [c["branch"] for _, c in members if c["branch"] is not None]
        line_avg = sum(lines) / len(lines) if lines else None
        branch_avg = sum(branches) / len(branches) if branches else None
        rows.append(f"| `{prefix}` | {len(members)} | {fmt(line_avg)} | {fmt(branch_avg)} | "
                    f"{rule.get('line', '-')} / {rule.get('branch', '-')} |")
        for rel, cov in members:
            if cov["line"] is not None and cov["line"] + 1e-9 < rule.get("line", 0):
                violations.append(f"{rel}: line coverage {fmt(cov['line'])} < {rule['line']}% ({prefix})")
            if "branch" in rule and cov["branch"] is not None and cov["branch"] + 1e-9 < rule["branch"]:
                violations.append(f"{rel}: branch coverage {fmt(cov['branch'])} < {rule['branch']}% ({prefix})")

    if args.baseline and args.changed and Path(args.baseline).exists():
        baseline = load_report(args.baseline, root, excludes)
        tolerance = float(config.get("max_changed_file_drop", 0.0))
        for rel in Path(args.changed).read_text().split():
            before, after = baseline.get(rel, {}).get("line"), files.get(rel, {}).get("line")
            if before is not None and after is not None and after + tolerance + 1e-9 < before:
                violations.append(f"{rel}: line coverage decreased {fmt(before)} -> {fmt(after)} (no-decrease rule)")

    md = ["## Coverage by module", "",
          "| Module | Files | Line | Branch | Threshold (line / branch) |", "|---|---|---|---|---|", *rows, ""]
    if violations:
        md += ["### Violations", ""] + [f"- {v}" for v in violations] + [""]
    else:
        md += ["All coverage thresholds met.", ""]
    markdown = "\n".join(md)
    print(markdown)
    if args.markdown:
        Path(args.markdown).write_text(markdown + "\n")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as fh:
            fh.write(markdown + "\n")
    for v in violations:
        print(f"::error title=Coverage gate::{v}")
    return 1 if violations else 0


if __name__ == "__main__":
    sys.exit(main())
