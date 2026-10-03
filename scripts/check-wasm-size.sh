#!/usr/bin/env bash
set -euo pipefail

# Find budget and tolerance from docs
DOC="docs/wasm-size-budget.md"
WASM_PATH="target/wasm32-unknown-unknown/release/ledgerlens_score.wasm"

# Optional wasm-opt evaluation (issue #1194). When WASM_OPT=1 the size gate is
# evaluated against a wasm-opt processed copy of the artifact so that the
# candidate pass pipeline is measured with the same budget/tolerance rules.
WASM_OPT="${WASM_OPT:-0}"
WASM_OPT_LEVEL="${WASM_OPT_LEVEL:--Oz}"
WASM_OPT_PASSES="${WASM_OPT_PASSES:-}"
WASM_OPT_BIN="${WASM_OPT_BIN:-wasm-opt}"

BUDGET_STR=$(grep "\- \*\*Total Binary Size\*\*:" "$DOC" | grep -oE '[0-9,]+ bytes' | grep -oE '[0-9,]+' | tr -d ',')
TOLERANCE_STR=$(grep "\- \*\*Tolerance\*\*:" "$DOC" | grep -oE '[0-9.]+%' | tr -d '%' || echo "0")

if [[ -z "$BUDGET_STR" ]]; then
  echo "Error: Could not extract budget from $DOC"
  exit 1
fi

BUDGET=$BUDGET_STR
TOLERANCE=${TOLERANCE_STR:-0}

if [[ ! -f "$WASM_PATH" ]]; then
  echo "WASM binary not found at $WASM_PATH. Building..."
  cargo build --target wasm32-unknown-unknown --release -p ledgerlens-score --locked
fi

# Baseline (pre-optimisation) size, always reported for comparison.
BASELINE=$(wc -c < "$WASM_PATH" | tr -d ' ')

MEASURED_PATH="$WASM_PATH"
if [[ "$WASM_OPT" == "1" ]]; then
  if ! command -v "$WASM_OPT_BIN" &> /dev/null; then
    echo "Error: WASM_OPT=1 but '$WASM_OPT_BIN' was not found on PATH."
    echo "Install Binaryen and record its version in the release manifest before adopting."
    exit 1
  fi

  OPT_PATH="${WASM_PATH%.wasm}.wasm-opt.wasm"
  echo "Running wasm-opt ($WASM_OPT_LEVEL) for size evaluation..."
  echo "  wasm-opt version: $("$WASM_OPT_BIN" --version 2>/dev/null || echo unknown)"

  OPT_ARGS=("$WASM_OPT_LEVEL")
  if [[ -n "$WASM_OPT_PASSES" ]]; then
    # shellcheck disable=SC2206
    OPT_ARGS+=($WASM_OPT_PASSES)
  fi

  "$WASM_OPT_BIN" "${OPT_ARGS[@]}" "$WASM_PATH" -o "$OPT_PATH"
  MEASURED_PATH="$OPT_PATH"
fi

ACTUAL=$(wc -c < "$MEASURED_PATH" | tr -d ' ')

MAX_ALLOWED=$(awk "BEGIN {print int($BUDGET * (1 + $TOLERANCE / 100))}")

echo "WASM Size Check:"
echo "  Actual: $ACTUAL bytes"
if [[ "$MEASURED_PATH" != "$WASM_PATH" ]]; then
  echo "  Baseline (pre-wasm-opt): $BASELINE bytes"
  echo "  wasm-opt delta: $((ACTUAL - BASELINE)) bytes"
fi
echo "  Budget: $BUDGET bytes"
echo "  Tolerance: $TOLERANCE%"
echo "  Max Allowed: $MAX_ALLOWED bytes"

if [[ "$ACTUAL" -gt "$MAX_ALLOWED" ]]; then
  DELTA=$((ACTUAL - BUDGET))
  echo ""
  echo "❌ ERROR: WASM size budget exceeded!"
  echo "Actual size ($ACTUAL bytes) exceeds the budget ($BUDGET bytes) + tolerance ($TOLERANCE%) by $((ACTUAL - MAX_ALLOWED)) bytes."
  echo "Delta from baseline: +$DELTA bytes."
  echo ""
  
  if ! command -v twiggy &> /dev/null; then
    echo "Installing twiggy for size analysis..."
    cargo install twiggy || echo "⚠️ WARNING: twiggy installation failed, skipping detailed breakdown."
  fi

  if command -v twiggy &> /dev/null; then
    echo "Detailed breakdown of current size:"
    ./scripts/wasm-size-report.sh --wasm "$MEASURED_PATH" --top 15 || echo "⚠️ WARNING: Detailed breakdown script failed."
  else
    echo "⚠️ WARNING: twiggy unavailable, skipping detailed breakdown."
  fi
  echo ""
  echo "To bypass this gate, you must explicitly update the budget in docs/wasm-size-budget.md and get it reviewed."
  exit 1
else
  echo "✅ WASM size is within budget."
fi
