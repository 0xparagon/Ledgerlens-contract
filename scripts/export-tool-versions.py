#!/usr/bin/env python3
"""Emit the shared toolchain manifest as GitHub Actions outputs."""

import json
from pathlib import Path
import re
import tomllib


versions = json.loads(Path("toolchain-versions.json").read_text(encoding="utf-8"))
rust_channel = tomllib.loads(Path("rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]["channel"].strip()
if rust_channel != versions["contract_rust"]:
    raise SystemExit(
        f"rust-toolchain.toml pins {rust_channel}, but toolchain-versions.json pins {versions['contract_rust']}"
    )

for manifest in Path("deploy/manifests").glob("*.env"):
    contents = manifest.read_text(encoding="utf-8")
    match = re.search(r'^EXPECTED_STELLAR_CLI_VERSION="([^"]+)"$', contents, re.MULTILINE)
    if not match:
        raise SystemExit(f"{manifest} is missing EXPECTED_STELLAR_CLI_VERSION")
    if match.group(1) != versions["stellar_cli"]:
        raise SystemExit(
            f"{manifest} pins Stellar CLI {match.group(1)}, expected {versions['stellar_cli']} from toolchain-versions.json"
        )

for key, value in versions.items():
    print(f"{key}={value}")