#!/usr/bin/env python3
"""Executable-documentation checker (issue #1230).

Extracts fenced code blocks from the README and the integration guides and
checks them against the contract source so documentation drift fails CI.

Checks performed
----------------
1. Signatures: every README heading of the form ``### `name(args) -> Ret` ``
   must match the parameter names/types and return type of
   ``pub fn name(env: Env, ...)`` in the score contract (``Result<T, Error>``
   is documented as ``T``).
2. Rust snippets: delimiters must balance, and every ``pub struct`` shown must
   list exactly the fields (name and type) of the struct in the source.
3. Shell snippets: syntax-checked with ``bash -n``; ``run`` snippets are
   executed in an empty temp dir with a scrubbed env and a timeout.

Snippet markers (fence info string, comma or space separated)
-------------------------------------------------------------
``compile`` (default) - static checks above only.
``run``               - shell only: also execute the snippet hermetically.
``ignore``            - skip the snippet; the line directly above the fence
                        must be ``<!-- snippet-ignore: <justification> -->``.

Usage: python3 tools/check_doc_snippets.py [--self-test]
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC_DIR = ROOT / "contracts" / "ledgerlens-score" / "src"
DOCS = [
    "README.md",
    "docs/amm-integration-guide.md",
    "docs/typed-client-examples.md",
    "docs/consumer-integration-decision-guide.md",
]
RUST_LANGS = {"rust", "rs"}
SHELL_LANGS = {"bash", "sh", "shell", "console"}
RUN_TIMEOUT_SECS = 30
FENCE = re.compile(r"^(```+|~~~+)\s*(.*)$")
IGNORE_NOTE = re.compile(r"^<!--\s*snippet-ignore:\s*(\S.*?)\s*-->$")
HEADING_SIG = re.compile(r"`([a-z_][a-z0-9_]*)\((.*?)\)(?:\s*->\s*([^`]+?))?`")
PLACEHOLDER = re.compile(r"<[A-Z][A-Z0-9_]*>")


def norm(ty: str) -> str:
    return re.sub(r"\s+", "", ty)


def split_top(s: str, sep: str = ",") -> list[str]:
    """Split on ``sep`` at nesting depth zero."""
    out, depth, cur = [], 0, ""
    for ch in s:
        if ch in "<([":
            depth += 1
        elif ch in ">)]":
            depth -= 1
        if ch == sep and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur)
    return [p.strip() for p in out if p.strip()]


def parse_params(s: str) -> list[tuple[str, str]]:
    params = []
    for p in split_top(s):
        name, _, ty = p.partition(":")
        params.append((name.strip().lstrip("_"), norm(ty)))
    return params


def load_source() -> str:
    return "\n".join(p.read_text() for p in sorted(SRC_DIR.glob("*.rs")) if not p.name.startswith("test_"))


def source_fns(src: str) -> dict[str, tuple[list[tuple[str, str]], str]]:
    fns = {}
    for m in re.finditer(r"pub fn ([a-z_][a-z0-9_]*)\s*\(\s*env:\s*Env\s*,?", src):
        i, depth = m.end(), 1
        start = i
        while depth:
            depth += {"(": 1, ")": -1}.get(src[i], 0)
            i += 1
        params = parse_params(src[start : i - 1])
        ret = re.match(r"\s*(?:->\s*([^{;]+))?", src[i:]).group(1) or "()"
        ret = norm(ret)
        r = re.fullmatch(r"Result<(.+),Error>", ret)
        fns.setdefault(m.group(1), (params, r.group(1) if r else ret))
    return fns


def struct_fields(body: str) -> list[tuple[str, str]]:
    body = re.sub(r"//[^\n]*", "", body)
    body = re.sub(r"#\[[^\]]*\]", "", body)
    fields = []
    for f in split_top(body):
        f = re.sub(r"^pub(\([^)]*\))?\s+", "", f.strip())
        name, _, ty = f.partition(":")
        fields.append((name.strip(), norm(ty)))
    return fields


def structs(src: str) -> dict[str, list[tuple[str, str]]]:
    out = {}
    for m in re.finditer(r"pub struct (\w+)\s*\{", src):
        i, depth = m.end(), 1
        while depth:
            depth += {"{": 1, "}": -1}.get(src[i], 0)
            i += 1
        out.setdefault(m.group(1), struct_fields(src[m.end() : i - 1]))
    return out


def balanced(code: str) -> bool:
    code = re.sub(r'"(\\.|[^"\\])*"', '""', re.sub(r"//[^\n]*", "", code))
    stack, pairs = [], {")": "(", "]": "[", "}": "{"}
    for ch in code:
        if ch in "([{":
            stack.append(ch)
        elif ch in pairs and (not stack or stack.pop() != pairs[ch]):
            return False
    return not stack


def snippets(text: str):
    """Yield (line_no, lang, markers, code, preceding_line)."""
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        m = FENCE.match(lines[i].strip())
        if not m:
            i += 1
            continue
        fence, info = m.group(1), m.group(2)
        tokens = [t for t in re.split(r"[,\s]+", info) if t]
        lang = tokens[0].lower() if tokens else ""
        start, prev = i + 1, lines[i - 1].strip() if i else ""
        i += 1
        body = []
        while i < len(lines) and not lines[i].strip().startswith(fence):
            body.append(lines[i])
            i += 1
        i += 1
        yield start, lang, set(tokens[1:]), "\n".join(body), prev


def run_shell(code: str) -> str | None:
    with tempfile.TemporaryDirectory() as tmp:
        env = {"PATH": "/usr/bin:/bin", "HOME": tmp, "LC_ALL": "C"}
        try:
            r = subprocess.run(
                ["bash", "-euo", "pipefail", "-c", code],
                cwd=tmp, env=env, capture_output=True, text=True, timeout=RUN_TIMEOUT_SECS,
            )
        except subprocess.TimeoutExpired:
            return f"timed out after {RUN_TIMEOUT_SECS}s"
    return None if r.returncode == 0 else f"exit {r.returncode}: {r.stderr.strip()[:300]}"


def check_sig(rel: str, n: int, m: re.Match, fns) -> list[str]:
    name, args, ret = m.group(1), m.group(2), norm(m.group(3) or "()")
    if name not in fns:
        return [f"{rel}:{n}: documented function `{name}` not found in contract"]
    sp, sr = fns[name]
    if parse_params(args) != sp or ret != sr:
        want = ", ".join(f"{a}: {t}" for a, t in sp)
        return [f"{rel}:{n}: signature drift for `{name}`; source is `{name}({want}) -> {sr}`"]
    return []


def check_doc(rel: str, text: str, fns, strs) -> list[str]:
    errs = []
    for n, line in enumerate(text.splitlines(), 1):
        if not re.match(r"#{2,4}\s+`", line):
            continue
        for m in HEADING_SIG.finditer(line):
            errs += check_sig(rel, n, m, fns)
 
    for n, lang, marks, code, prev in snippets(text):
        where = f"{rel}:{n}"
        if "ignore" in marks:
            if not IGNORE_NOTE.match(prev):
                errs.append(f"{where}: `ignore` snippet needs `<!-- snippet-ignore: reason -->` above the fence")
            continue
        if lang in RUST_LANGS:
            if "run" in marks:
                errs.append(f"{where}: `run` is only supported for shell snippets")
            if not balanced(code):
                errs.append(f"{where}: unbalanced delimiters in rust snippet")
            for m in re.finditer(r"pub struct (\w+)\s*\{", code):
                j, depth = m.end(), 1
                while j < len(code) and depth:
                    depth += {"{": 1, "}": -1}.get(code[j], 0)
                    j += 1
                name = m.group(1)
                if name in strs and struct_fields(code[m.end() : j - 1]) != strs[name]:
                    want = ", ".join(f"{a}: {t}" for a, t in strs[name])
                    errs.append(f"{where}: struct `{name}` drift; source fields are {{ {want} }}")
        elif lang in SHELL_LANGS:
            if lang == "console":
                code = "\n".join(l[2:] for l in code.splitlines() if l.startswith("$ "))
            code = PLACEHOLDER.sub("PLACEHOLDER", code)
            r = subprocess.run(["bash", "-n"], input=code, capture_output=True, text=True)
            if r.returncode:
                errs.append(f"{where}: shell syntax error: {r.stderr.strip()}")
            elif "run" in marks and (e := run_shell(code)):
                errs.append(f"{where}: run failed: {e}")
    return errs


def self_test() -> None:
    src = "pub fn f(env: Env, a: u32, b: Vec<Address>) -> Result<u32, Error> {}\npub struct S { pub a: u32, pub b: Option<Bytes>, }"
    fns, strs = source_fns(src), structs(src)
    assert check_doc("t", "### `f(a: u32, b: Vec<Address>) -> u32` / `f() -> u32`\n", fns, strs)
    ok = "### `f(a: u32, b: Vec<Address>) -> u32`\n```rust\npub struct S {\n    pub a: u32, // c\n    pub b: Option<Bytes>,\n}\n```\n"
    assert check_doc("t", ok, fns, strs) == [], check_doc("t", ok, fns, strs)
    assert check_doc("t", "### `f(a: u64, b: Vec<Address>) -> u32`\n", fns, strs)
    assert check_doc("t", "```rust\npub struct S { pub a: u32 }\n```\n", fns, strs)
    assert check_doc("t", "```bash ignore\necho\n```\n", fns, strs)
    assert not check_doc("t", "<!-- snippet-ignore: needs network -->\n```bash ignore\ncurl x\n```\n", fns, strs)
    assert check_doc("t", "```bash\nif then\n```\n", fns, strs)
    assert not check_doc("t", "```bash run\necho hi\n```\n", fns, strs)
    assert check_doc("t", "```bash run\nexit 3\n```\n", fns, strs)
    print("self-test ok")


def main() -> int:
    if "--self-test" in sys.argv:
        self_test()
        return 0
    src = load_source()
    fns, strs = source_fns(src), structs(src)
    errs = []
    for rel in DOCS:
        path = ROOT / rel
        if path.exists():
            errs += check_doc(rel, path.read_text(), fns, strs)
    for e in errs:
        print(f"error: {e}", file=sys.stderr)
    print(f"checked {len(DOCS)} documents: {len(errs)} problem(s)")
    return 1 if errs else 0


if __name__ == "__main__":
    sys.exit(main())
