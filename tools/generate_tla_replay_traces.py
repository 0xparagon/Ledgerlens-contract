#!/usr/bin/env python3
"""Generate structured replay inputs from TLC simulation trace modules."""

import argparse
import json
import re
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SPEC_DIR = ROOT / "spec"
TRACE_HEADER = re.compile(r"^\\\* <(.*?) line \d+, col \d+ to line \d+, col \d+ of module [A-Za-z][A-Za-z0-9_]*>", re.M)
STATE_HEADER = re.compile(r"^STATE_(\d+) ==\s*$", re.M)
ASSIGNMENT = re.compile(r"^/\\\s+([a-z][a-z0-9_]*)\s*=\s*(.*)$", re.M)
ACTION = re.compile(r"^([A-Za-z][A-Za-z0-9_]*)(?:\((.*)\))?$", re.S)


def split_items(value: str) -> list[str]:
    items = []
    start = 0
    depth = 0
    quoted = False
    escaped = False
    for index, char in enumerate(value):
        if quoted:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                quoted = False
        elif char == '"':
            quoted = True
        elif char in "{[":
            depth += 1
        elif char in "}]":
            depth -= 1
        elif char == "," and depth == 0:
            items.append(value[start:index].strip())
            start = index + 1
    if value[start:].strip():
        items.append(value[start:].strip())
    return items


def parse_value(value: str):
    value = value.strip()
    if value == "TRUE":
        return True
    if value == "FALSE":
        return False
    if re.fullmatch(r"-?\d+", value):
        return int(value)
    if value.startswith('"'):
        return json.loads(value)
    if value.startswith("{") and value.endswith("}"):
        inner = value[1:-1].strip()
        return [] if not inner else [parse_value(item) for item in split_items(inner)]
    if value.startswith("[") and value.endswith("]"):
        inner = value[1:-1].strip()
        result = {}
        for item in split_items(inner):
            key, separator, item_value = item.partition("|->")
            if not separator:
                raise ValueError(f"cannot parse TLC function entry {item!r}")
            key_value = parse_value(key.strip())
            result[str(key_value)] = parse_value(item_value.strip())
        return result
    if re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*", value):
        return value
    raise ValueError(f"unsupported TLC value syntax: {value!r}")


def parse_trace(path: Path) -> dict:
    text = path.read_text(encoding="utf-8")
    states = list(STATE_HEADER.finditer(text))
    actions = list(TRACE_HEADER.finditer(text))
    if not states or len(actions) != len(states):
        raise ValueError(f"unexpected TLC trace module layout in {path}")

    parsed_states = []
    for index, state_match in enumerate(states):
        end = states[index + 1].start() if index + 1 < len(states) else len(text)
        block = text[state_match.end():end]
        action_match = ACTION.fullmatch(actions[index].group(1).strip())
        if action_match is None:
            raise ValueError(f"cannot parse TLC action label {actions[index].group(1)!r}")
        variables = {
            name: value.strip()
            for name, value in ASSIGNMENT.findall(block)
        }
        action_name = action_match.group(1)
        if action_name.startswith("Replay"):
            action_name = action_name[len("Replay"):]
        arguments_text = action_match.group(2) or ""
        parsed_states.append({
            "step": int(state_match.group(1)) - 1,
            "action": action_name,
            "arguments": [parse_value(item) for item in split_items(arguments_text)],
            "variables": variables,
        })
        parsed_states[-1]["variables"] = {
            name: parse_value(value) for name, value in variables.items()
        }
        if action_name == "ExecuteGov":
            break

    if parsed_states[0]["action"] != "Init":
        raise ValueError(f"TLC trace {path} does not begin with Init")
    return {"behavior_id": path.name, "states": parsed_states}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", type=Path, default=SPEC_DIR / "tla2tools.jar")
    parser.add_argument("--config", type=Path, default=SPEC_DIR / "LedgerLensReplay.cfg")
    parser.add_argument("--module", default="LedgerLensReplay.tla")
    parser.add_argument("--traces", type=int, default=20)
    parser.add_argument("--depth", type=int, default=12)
    parser.add_argument("--seed", type=int, default=17)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    if not args.jar.is_file():
        parser.error(f"TLC jar not found: {args.jar}")

    with tempfile.TemporaryDirectory(prefix="ledgerlens-tlc-") as temp_dir:
        prefix = Path(temp_dir) / "trace"
        command = [
            "java", "-XX:+UseParallelGC", "-jar", str(args.jar),
            "-simulate", f"file={prefix},num={args.traces}",
            "-depth", str(args.depth), "-seed", str(args.seed),
            "-config", str(args.config), args.module,
        ]
        subprocess.run(command, cwd=SPEC_DIR, check=True)
        files = sorted(Path(temp_dir).glob("trace_*"))
        if not files:
            raise RuntimeError("TLC completed without emitting any trace modules")
        behaviors = [parse_trace(path) for path in files]

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(behaviors, indent=2) + "\n", encoding="utf-8")
    print(f"Parsed {len(behaviors)} TLC behaviors into {args.output}")
    print(f"Transitions: {sum(len(item['states']) - 1 for item in behaviors)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())