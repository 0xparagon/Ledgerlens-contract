#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
tools/check_contract_build_lints.sh
cargo build --target wasm32-unknown-unknown --release -p ledgerlens-score --locked
scripts/check-wasm-size.sh
cargo run -p schema-gen -- --wasm target/wasm32-unknown-unknown/release/ledgerlens_score.wasm \
  --invocations --out target/invocation-fuzzer
cargo run -p invocation-fuzzer --locked -- smoke --abi-schema \
  target/invocation-fuzzer/ledgerlens_score.invocations.schema.json

TLA_TOOLS_JAR="${TLA_TOOLS_JAR:-$ROOT/.cache/tla2tools.jar}"
if [[ ! -f "$TLA_TOOLS_JAR" ]]; then
  printf 'TLA+ tools jar not found at %s (set TLA_TOOLS_JAR to an installed jar)\n' "$TLA_TOOLS_JAR" >&2
  exit 1
fi
(cd spec && java -XX:+UseParallelGC -jar "$TLA_TOOLS_JAR" -simulate num=20000 -depth 30 -config LedgerLens.cfg LedgerLens.tla)