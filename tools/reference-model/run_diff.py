#!/usr/bin/env python3
"""Differential runner (issue #1240): generate op sequences, run them through
the reference model and the contract driver, and report disagreements.

Usage: python3 run_diff.py --sequences 100000 --seed 1 [--driver PATH]
Any disagreement fails the run and prints the shortest failing prefix. Record
and classify it in DISAGREEMENTS.md before fixing the code, docs, or model.
"""

from __future__ import annotations

import argparse
import json
import random
import subprocess
import sys
from collections import Counter
from pathlib import Path

import ledgerlens_ref as ref

HERE = Path(__file__).resolve().parent
PAIRS = ["XLM_USDC", "BTC_USDC", "ETH_XLM"]


def gen_op(rng: random.Random) -> dict:
    w, p = rng.randrange(3), rng.choice(PAIRS)
    k = rng.choices(
        ["advance", "submit", "weight", "decay", "floor", "get", "eff", "agg", "gate"],
        [2, 5, 1, 1, 1, 2, 2, 2, 3],
    )[0]
    if k == "advance":
        return {"op": k, "secs": rng.choice([3_601, 86_400, 604_801, 2_592_000])}
    if k == "submit":
        return {"op": k, "w": w, "p": p, "score": rng.choice([0, 1, 50, 99, 100, 101, rng.randrange(120)]),
                "conf": rng.choice([0, 50, 100, 101, rng.randrange(120)])}
    if k == "weight":
        return {"op": k, "p": p, "weight": rng.choice([0, 1, 2, 5, 10])}
    if k == "decay":
        return {"op": k, "num": rng.choice([0, 1, 2]), "den": rng.choice([1_000_000, 2_592_000, 10_000_000])}
    if k == "floor":
        return {"op": k, "v": rng.choice([0, 50, 75, 100, 101])}
    if k in ("get", "eff"):
        return {"op": k, "w": w, "p": p}
    if k == "agg":
        return {"op": k, "w": w}
    return {"op": k, "w": w, "p": p, "t": rng.choice([0, 40, 70, 100]), "q": rng.choice([0, 50, 80, 100])}


def gen_seq(rng: random.Random) -> list[dict]:
    ops = []
    for _ in range(rng.randrange(1, 16)):
        op = gen_op(rng)
        if op["op"] == "submit":  # stay clear of the documented 1-hour cooldown
            ops.append({"op": "advance", "secs": 3_601})
        ops.append(op)
    return ops


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--sequences", type=int, default=1000)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--driver", default=str(HERE.parents[1] / "target/release/reference-model-driver"))
    args = ap.parse_args()

    rng = random.Random(args.seed)
    seqs = [gen_seq(rng) for _ in range(args.sequences)]
    proc = subprocess.run([args.driver], input="\n".join(json.dumps(s) for s in seqs) + "\n",
                          capture_output=True, text=True, check=True)
    outs = proc.stdout.splitlines()
    assert len(outs) == len(seqs), "driver output length mismatch"

    counts: Counter[str] = Counter()
    first: dict[str, tuple] = {}
    for seq, line in zip(seqs, outs):
        for i, (op, m, c) in enumerate(zip(seq, ref.run(seq), json.loads(line))):
            if m != c:
                counts[op["op"]] += 1
                prev = first.get(op["op"])
                if prev is None or i + 1 < len(prev[0]):
                    first[op["op"]] = (seq[: i + 1], m, c)
                break  # later ops depend on earlier state; count once per sequence

    print(f"{len(seqs)} sequences, seed {args.seed}: {sum(counts.values())} disagreeing")
    for kind, n in sorted(counts.items()):
        ops, m, c = first[kind]
        print(f"  {kind}: {n} sequences; shortest repro: model={m} contract={c}\n    {json.dumps(ops)}")
    return 1 if counts else 0


if __name__ == "__main__":
    sys.exit(main())
