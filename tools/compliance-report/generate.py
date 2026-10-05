#!/usr/bin/env python3
"""Deterministic compliance and governance activity report (issue #1222).

Reads a JSON-lines event corpus exported by the indexer (see
docs/compliance-report.md for the record schema) and writes:

  <out>/report.json   canonical JSON report, including `content_hash`
  <out>/report.html   static HTML rendering of the same data

Determinism: the report depends only on the corpus and the CLI arguments. No
wall-clock time, locale, hostname, environment or dict-ordering leaks in; all
numbers are integers; JSON is serialised canonically (sorted keys, compact
separators, ASCII only, trailing newline). `content_hash` is the SHA-256 of
the canonical JSON of the report with the `content_hash` field removed, so
two parties with the same inputs obtain byte-identical files and hashes.

Privacy: wallet-level data never appears in the output. See the redaction
policy in docs/compliance-report.md.
"""

import argparse
import hashlib
import html
import json
import sys
from pathlib import Path

REPORT_VERSION = 1
SCORE_MIN, SCORE_MAX = 0, 100
KNOWN_TYPES = {"governance_action", "signer_change", "param_change", "heartbeat", "score"}


def canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def load_corpus(path):
    events = []
    for lineno, line in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        ev = json.loads(line)
        if ev.get("type") not in KNOWN_TYPES:
            raise ValueError(f"{path}:{lineno}: unknown event type {ev.get('type')!r}")
        if not isinstance(ev.get("ledger"), int) or not isinstance(ev.get("contract_id"), str):
            raise ValueError(f"{path}:{lineno}: `ledger` (int) and `contract_id` (str) are required")
        events.append(ev)
    # Stable total order independent of corpus line order.
    events.sort(key=lambda e: (e["ledger"], e.get("event_index", 0), e["contract_id"], canonical(e)))
    return events


def lineage(proposal_id, parents):
    chain, seen = [], set()
    while proposal_id is not None and proposal_id not in seen:
        seen.add(proposal_id)
        chain.append(proposal_id)
        proposal_id = parents.get(proposal_id)
    return list(reversed(chain))


def governance_section(events):
    parents = {e["proposal_id"]: e.get("parent_proposal_id") for e in events}
    return [{
        "ledger": e["ledger"],
        "contract_id": e["contract_id"],
        "proposal_id": e["proposal_id"],
        "action": e["action"],
        "actor": e["actor"],
        "tx_hash": e.get("tx_hash"),
        "lineage": lineage(e["proposal_id"], parents),
    } for e in events]


def signer_section(events):
    history, active = [], {}
    for e in events:
        signers = active.setdefault(e["contract_id"], set())
        if e["change"] == "added":
            signers.add(e["signer"])
        elif e["change"] == "removed":
            signers.discard(e["signer"])
        history.append({
            "ledger": e["ledger"],
            "contract_id": e["contract_id"],
            "signer": e["signer"],
            "change": e["change"],
            "active_signer_count": len(signers),
        })
    return {"history": history, "final_active": {c: sorted(s) for c, s in sorted(active.items())}}


def parameter_section(events):
    provenance, current = [], {}
    for e in events:
        key = (e["contract_id"], e["param"])
        provenance.append({
            "ledger": e["ledger"],
            "contract_id": e["contract_id"],
            "param": e["param"],
            "old": e.get("old", current.get(key)),
            "new": e["new"],
            "proposal_id": e.get("proposal_id"),
        })
        current[key] = e["new"]
    return {"changes": provenance,
            "current": [{"contract_id": c, "param": p, "value": v} for (c, p), v in sorted(current.items())]}


def heartbeat_section(events, ledger_from, ledger_to, max_gap):
    by_contract = {}
    for e in events:
        by_contract.setdefault(e["contract_id"], []).append(e["ledger"])
    out = {}
    for contract, ledgers in sorted(by_contract.items()):
        points = [ledger_from] + sorted(set(ledgers)) + [ledger_to]
        downtime = [{"from_ledger": a, "to_ledger": b, "gap_ledgers": b - a}
                    for a, b in zip(points, points[1:]) if b - a > max_gap]
        out[contract] = {
            "heartbeats": len(set(ledgers)),
            "downtime_intervals": downtime,
            "downtime_ledgers": sum(d["gap_ledgers"] for d in downtime),
        }
    return out


def score_section(events, bucket_width, k):
    n_buckets = -(-SCORE_MAX // bucket_width)  # ceil; the last bucket also holds SCORE_MAX
    per_contract = {}
    for e in events:
        score = e["score"]
        if not isinstance(score, int) or not SCORE_MIN <= score <= SCORE_MAX:
            raise ValueError(f"score out of range at ledger {e['ledger']}: {score!r}")
        counts = per_contract.setdefault(e["contract_id"], [0] * n_buckets)
        counts[min(score // bucket_width, n_buckets - 1)] += 1
    out = {}
    for contract, counts in sorted(per_contract.items()):
        buckets = []
        for b, n in enumerate(counts):
            lo = b * bucket_width
            hi = SCORE_MAX if b == n_buckets - 1 else lo + bucket_width - 1
            hidden = 0 < n < k
            buckets.append({"range": [lo, hi], "count": None if hidden else n, "suppressed": hidden})
        # Complementary suppression: with exactly one hidden bucket its count
        # would follow from the total, so also hide the smallest visible one.
        if sum(b["suppressed"] for b in buckets) == 1:
            visible = [b for b in buckets if not b["suppressed"] and b["count"]]
            if visible:
                smallest = min(visible, key=lambda b: (b["count"], b["range"][0]))
                smallest["count"], smallest["suppressed"] = None, True
        total = sum(counts)
        out[contract] = {
            "total_scores": total if total >= k else None,
            "suppressed_buckets": sum(b["suppressed"] for b in buckets),
            "buckets": buckets,
        }
    return out


def build_report(events, ledger_from, ledger_to, contracts, bucket_width, k, max_gap):
    in_scope = [e for e in events
                if ledger_from <= e["ledger"] <= ledger_to and (not contracts or e["contract_id"] in contracts)]
    by_type = {t: [e for e in in_scope if e["type"] == t] for t in sorted(KNOWN_TYPES)}
    report = {
        "report_version": REPORT_VERSION,
        "scope": {
            "ledger_from": ledger_from,
            "ledger_to": ledger_to,
            "contract_ids": sorted(contracts) if contracts else sorted({e["contract_id"] for e in in_scope}),
            "event_count": len(in_scope),
        },
        "privacy": {"min_bucket_size_k": k, "score_bucket_width": bucket_width, "wallet_identifiers": "omitted"},
        "governance_actions": governance_section(by_type["governance_action"]),
        "signer_set": signer_section(by_type["signer_change"]),
        "parameters": parameter_section(by_type["param_change"]),
        "heartbeat": heartbeat_section(by_type["heartbeat"], ledger_from, ledger_to, max_gap),
        "score_distribution": score_section(by_type["score"], bucket_width, k),
    }
    report["content_hash"] = hashlib.sha256(canonical(report).encode("ascii")).hexdigest()
    return report


def _table(headers, rows):
    esc = lambda v: html.escape("—" if v is None else (" → ".join(map(str, v)) if isinstance(v, list) else str(v)))
    head = "".join(f"<th>{html.escape(h)}</th>" for h in headers)
    body = "".join("<tr>" + "".join(f"<td>{esc(c)}</td>" for c in r) + "</tr>" for r in rows)
    return f"<table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table>"


def render_html(r):
    s = r["scope"]
    parts = [
        "<!doctype html>",
        '<html lang="en"><head><meta charset="utf-8">',
        "<title>LedgerLens Compliance Report</title>",
        "<style>body{font-family:system-ui,sans-serif;max-width:960px;margin:2rem auto;padding:0 1rem}"
        "table{border-collapse:collapse;width:100%;margin:.5rem 0 1.5rem}td,th{border:1px solid #ccc;"
        "padding:.25rem .5rem;text-align:left;font-size:.9rem}code{word-break:break-all}</style>",
        "</head><body>",
        "<h1>LedgerLens Compliance &amp; Governance Report</h1>",
        f"<p>Ledgers <b>{s['ledger_from']}</b>–<b>{s['ledger_to']}</b> · {s['event_count']} events · "
        f"contracts: {html.escape(', '.join(s['contract_ids']))}</p>",
        f"<p>Content hash (SHA-256): <code>{r['content_hash']}</code></p>",
        "<h2>Governance actions</h2>",
        _table(["Ledger", "Contract", "Proposal", "Action", "Actor", "Lineage"],
               [[g["ledger"], g["contract_id"], g["proposal_id"], g["action"], g["actor"], g["lineage"]]
                for g in r["governance_actions"]]),
        "<h2>Signer set history</h2>",
        _table(["Ledger", "Contract", "Signer", "Change", "Active signers"],
               [[h["ledger"], h["contract_id"], h["signer"], h["change"], h["active_signer_count"]]
                for h in r["signer_set"]["history"]]),
        "<h2>Parameter provenance</h2>",
        _table(["Ledger", "Contract", "Parameter", "Old", "New", "Proposal"],
               [[p["ledger"], p["contract_id"], p["param"], p["old"], p["new"], p["proposal_id"]]
                for p in r["parameters"]["changes"]]),
        "<h2>Heartbeat and downtime</h2>",
        _table(["Contract", "Heartbeats", "Downtime ledgers", "Intervals"],
               [[c, h["heartbeats"], h["downtime_ledgers"],
                 "; ".join(f"{d['from_ledger']}–{d['to_ledger']}" for d in h["downtime_intervals"]) or None]
                for c, h in r["heartbeat"].items()]),
        f"<h2>Score distribution</h2><p>Buckets with fewer than {r['privacy']['min_bucket_size_k']} "
        "scores are suppressed.</p>",
    ]
    for contract, d in r["score_distribution"].items():
        parts.append(f"<h3>{html.escape(contract)}</h3>")
        parts.append(_table(["Range", "Count"],
                            [[f"{b['range'][0]}–{b['range'][1]}", "suppressed" if b["suppressed"] else b["count"]]
                             for b in d["buckets"]]))
    parts.append("</body></html>")
    return "\n".join(parts) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("corpus", help="JSON-lines event corpus")
    ap.add_argument("--out", required=True, help="output directory")
    ap.add_argument("--ledger-from", type=int, required=True)
    ap.add_argument("--ledger-to", type=int, required=True)
    ap.add_argument("--contract", action="append", default=[], help="restrict to contract id (repeatable)")
    ap.add_argument("--bucket-width", type=int, default=10)
    ap.add_argument("--k", type=int, default=5, help="minimum bucket size before suppression")
    ap.add_argument("--max-heartbeat-gap", type=int, default=720, help="ledgers (~1h at 5s/ledger)")
    args = ap.parse_args(argv)
    if args.ledger_from > args.ledger_to:
        ap.error("--ledger-from must be <= --ledger-to")

    report = build_report(load_corpus(args.corpus), args.ledger_from, args.ledger_to,
                          set(args.contract), args.bucket_width, args.k, args.max_heartbeat_gap)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "report.json").write_bytes((canonical(report) + "\n").encode("ascii"))
    (out / "report.html").write_bytes(render_html(report).encode("utf-8"))
    print(report["content_hash"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
