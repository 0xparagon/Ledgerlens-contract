#!/usr/bin/env python3
"""Generate bounded TLC behaviors and replay a minimal coverage-complete set."""

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
GENERATOR = ROOT / "tools" / "generate_tla_replay_traces.py"
REQUIRED_ACTIONS = {
    "SubmitScore",
    "PauseContract",
    "UnpauseContract",
    "MutateAdminSet",
    "ProposeGov",
    "ExecuteGov",
}


def action_set(behavior: dict) -> set[str]:
    return {state["action"] for state in behavior["states"]}


def choose_covering_behaviors(behaviors: list[dict]) -> list[dict]:
    remaining = set(REQUIRED_ACTIONS)
    selected = []
    candidates = list(behaviors)
    while remaining:
        best = max(
            candidates,
            key=lambda behavior: (
                len(action_set(behavior) & remaining),
                -len(behavior["states"]),
            ),
        )
        covered = action_set(best) & remaining
        if not covered:
            raise RuntimeError(f"TLC behaviors did not cover actions: {sorted(remaining)}")
        selected.append(best)
        candidates.remove(best)
        remaining -= covered
    return selected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", type=Path, default=ROOT / "spec" / "tla2tools.jar")
    parser.add_argument("--traces", type=int, default=50)
    parser.add_argument("--depth", type=int, default=16)
    parser.add_argument("--seed", type=int, default=17)
    parser.add_argument("--max-seed-attempts", type=int, default=20)
    args = parser.parse_args()

    with tempfile.TemporaryDirectory(prefix="ledgerlens-tla-replay-") as temp_dir:
        selected_path = Path(temp_dir) / "selected.json"
        selected = None
        selected_seed = None
        for seed in range(args.seed, args.seed + args.max_seed_attempts):
            generated_path = Path(temp_dir) / f"generated-{seed}.json"
            subprocess.run(
                [
                    sys.executable,
                    str(GENERATOR),
                    "--jar", str(args.jar.resolve()),
                    "--traces", str(args.traces),
                    "--depth", str(args.depth),
                    "--seed", str(seed),
                    "--output", str(generated_path),
                ],
                cwd=ROOT,
                check=True,
            )
            try:
                selected = choose_covering_behaviors(json.loads(generated_path.read_text()))
                selected_seed = seed
                break
            except RuntimeError as error:
                print(f"Seed {seed} missed required action coverage: {error}")
        if selected is None:
            raise RuntimeError("TLC did not produce required replay action coverage")
        selected_path.write_text(json.dumps(selected, indent=2) + "\n", encoding="utf-8")
        actions = sorted(set().union(*(action_set(behavior) for behavior in selected)))
        print(f"Replaying {len(selected)} TLC behavior(s) from seed {selected_seed}: {', '.join(actions)}")

        env = os.environ.copy()
        env["LEDGERLENS_TLA_TRACE_FILE"] = str(selected_path)
        result = subprocess.run(
            ["cargo", "test", "-p", "ledgerlens-score", "test_tla_trace_validation", "--", "--ignored", "--nocapture"],
            cwd=ROOT,
            env=env,
            check=False,
        )
        return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())