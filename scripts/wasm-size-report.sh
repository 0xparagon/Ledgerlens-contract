#!/usr/bin/env bash
set -euo pipefail

TOP=10
OUTPUT=""
WASM_PATH="target/wasm32-unknown-unknown/release/ledgerlens_score.wasm"
WASM_OPT=""
WASM_OPT_PASSES=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --top)
      TOP="$2"
      shift 2
      ;;
    --output)
      OUTPUT="$2"
      shift 2
      ;;
    --wasm)
      WASM_PATH="$2"
      shift 2
      ;;
    --wasm-opt)
      WASM_OPT="$2"
      shift 2
      ;;
    --wasm-opt-passes)
      WASM_OPT_PASSES="$2"
      shift 2
      ;;
    -h|--help)
      echo "Usage: $0 [--top <N>] [--output <PATH>] [--wasm <PATH>] [--wasm-opt <PATH>] [--wasm-opt-passes <PASSES>]"
      echo ""
      echo "  --wasm-opt <PATH>          Path to the wasm-opt binary (default: wasm-opt on PATH)."
      echo "  --wasm-opt-passes <PASSES> Comma-separated Binaryen pass pipeline to evaluate."
      echo "                             When set, the report includes a size comparison of the"
      echo "                             optimised artifact against the unoptimised build."
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [[ ! -f "$WASM_PATH" ]]; then
  echo "WASM binary not found at $WASM_PATH. Building..."
  cargo build --target wasm32-unknown-unknown --release -p ledgerlens-score
fi

if ! command -v twiggy &> /dev/null; then
  echo "Error: twiggy is not installed. Install it with: cargo install twiggy" >&2
  exit 1
fi

# Resolve the wasm-opt binary when a pass pipeline was requested.
WASM_OPT_BIN=""
if [[ -n "$WASM_OPT_PASSES" ]]; then
  if [[ -n "$WASM_OPT" ]]; then
    WASM_OPT_BIN="$WASM_OPT"
  elif command -v wasm-opt &> /dev/null; then
    WASM_OPT_BIN="wasm-opt"
  else
    echo "Error: wasm-opt not found. Install Binaryen or pass --wasm-opt <PATH>." >&2
    exit 1
  fi
fi

# Emit the optimised artifact next to the input so the report can compare sizes.
OPT_WASM_PATH=""
if [[ -n "$WASM_OPT_PASSES" ]]; then
  OPT_WASM_PATH="${WASM_PATH%.wasm}.wasm-opt.wasm"
fi

generate_report() {
  echo "# WASM Binary Size Report: ledgerlens-score.wasm"
  echo ""
  echo "Binary: $WASM_PATH"
  echo "Binary Size: $(wc -c < "$WASM_PATH" | tr -d ' ') bytes"
  echo ""

  if [[ -n "$WASM_OPT_PASSES" ]]; then
    echo "## 0. wasm-opt Pass Pipeline Evaluation"
    echo ""
    echo "Tool: $WASM_OPT_BIN"
    echo "Tool Version: $("$WASM_OPT_BIN" --version 2>/dev/null || echo unknown)"
    echo "Passes: $WASM_OPT_PASSES"
    echo ""
    echo "\`\`\`"
    "$WASM_OPT_BIN" "$WASM_PATH" -o "$OPT_WASM_PATH" --"$WASM_OPT_PASSES"
    echo "\`\`\`"
    echo ""
    local orig_size opt_size
    orig_size=$(wc -c < "$WASM_PATH" | tr -d ' ')
    opt_size=$(wc -c < "$OPT_WASM_PATH" | tr -d ' ')
    echo "| Artifact | Size (bytes) |"
    echo "| --- | --- |"
    echo "| unoptimised | $orig_size |"
    echo "| wasm-opt ($WASM_OPT_PASSES) | $opt_size |"
    echo ""
    echo "Optimised artifact: $OPT_WASM_PATH"
    echo ""
    echo "Note: semantic equivalence must be verified by running the full test-suite and"
    echo "the replay regression corpus against the optimised artifact before adoption."
    echo ""
  fi

  echo "## 1. Top Shallow Size Contributors (twiggy top)"
  echo ""
  echo "\`\`\`"
  twiggy top -n "$TOP" "$WASM_PATH"
  echo "\`\`\`"
  echo ""
  echo "## 2. Top Retained Size / Dominator Tree (twiggy dominators)"
  echo ""
  echo "\`\`\`"
  twiggy dominators -r "$TOP" "$WASM_PATH"
  echo "\`\`\`"
}

if [[ -n "$OUTPUT" ]]; then
  mkdir -p "$(dirname "$OUTPUT")"
  generate_report > "$OUTPUT"
  echo "WASM size report written to: $OUTPUT"
else
  generate_report
fi
