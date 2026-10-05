#!/usr/bin/env bash
# Upgrade the staging ledgerlens-score instance to a new WASM through the
# standard propose_upgrade/execute_upgrade flow, verify it, and roll back to
# the previous WASM if any check fails.
#
# Usage:
#   ./scripts/staging-deploy.sh [--dry-run] <network> <contract-id> <source-identity> <optimized-wasm> <status-file>
#
# The source identity must be the staging-only admin key (see
# docs/staging-deployment.md). Staging is configured with a zero upgrade
# timelock so execute_upgrade can follow propose_upgrade immediately.
#
# Writes a machine-readable status JSON to <status-file>.
# Exit codes: 0 deployed and healthy, 1 deploy failed (rolled back),
#             3 deploy failed AND rollback failed (manual action required).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DRY_RUN=false
POSITIONAL=()
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=true ;;
    --help) sed -n '2,15p' "$0"; exit 0 ;;
    *) POSITIONAL+=("$arg") ;;
  esac
done
[ "${#POSITIONAL[@]}" -eq 5 ] || { sed -n '6,7p' "$0" >&2; exit 2; }
NETWORK="${POSITIONAL[0]}"; CONTRACT_ID="${POSITIONAL[1]}"; SOURCE="${POSITIONAL[2]}"
WASM="${POSITIONAL[3]}"; STATUS_FILE="${POSITIONAL[4]}"

run() { if [ "$DRY_RUN" = true ]; then echo "[dry-run] $*"; else "$@"; fi; }

invoke() {
  run stellar contract invoke --id "$CONTRACT_ID" --source "$SOURCE" --network "$NETWORK" -- "$@"
}

onchain_hash() {
  if [ "$DRY_RUN" = true ]; then echo "${DRY_RUN_ONCHAIN_HASH:-0000}"; return; fi
  local tmp; tmp="$(mktemp)"
  stellar contract fetch --id "$CONTRACT_ID" --network "$NETWORK" --out-file "$tmp"
  sha256sum "$tmp" | awk '{print $1}'; rm -f "$tmp"
}

upgrade_to() {
  local hash="$1"
  invoke propose_upgrade --admin_signers "[\"$ADMIN\"]" --new_wasm_hash "$hash" &&
    invoke execute_upgrade --admin_signers "[\"$ADMIN\"]"
}

# Health check (read-only) + canary: on-chain code must equal the expected hash.
verify() {
  local expected="$1"
  run "$SCRIPT_DIR/health_check.sh" "$NETWORK" "$CONTRACT_ID" "$SOURCE" || return 1
  if [ "$DRY_RUN" != true ] || [ -n "${DRY_RUN_FAIL_VERIFY:-}" ]; then
    [ -z "${DRY_RUN_FAIL_VERIFY:-}" ] || return 1
    [ "$(onchain_hash)" = "$expected" ] || { echo "FAIL  on-chain hash != $expected"; return 1; }
  fi
}

write_status() {
  jq -n --arg network "$NETWORK" --arg contract "$CONTRACT_ID" \
    --arg status "$1" --arg wasm "$2" --arg previous "$PREVIOUS_HASH" \
    --arg commit "${GITHUB_SHA:-$(git rev-parse HEAD 2>/dev/null || echo unknown)}" \
    --arg version "$(grep -m1 '^version' "$SCRIPT_DIR/../contracts/ledgerlens-score/Cargo.toml" | cut -d'"' -f2)" \
    --arg run_url "${GITHUB_SERVER_URL:-}/${GITHUB_REPOSITORY:-}/actions/runs/${GITHUB_RUN_ID:-}" \
    --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{network:$network, contracts:{"ledgerlens-score":$contract}, version:$version,
      commit:$commit, wasm_sha256:$wasm, previous_wasm_sha256:$previous,
      last_deployment:{status:$status, at:$at, run_url:$run_url}}' > "$STATUS_FILE"
  cat "$STATUS_FILE"
}

ADMIN="$(run stellar keys address "$SOURCE" | tail -n1)"
PREVIOUS_HASH="$(onchain_hash)"
NEW_HASH="$(sha256sum "$WASM" | awk '{print $1}')"
echo "INFO  previous=$PREVIOUS_HASH new=$NEW_HASH"

if [ "$PREVIOUS_HASH" = "$NEW_HASH" ]; then
  echo "INFO  staging already runs this WASM; verifying only"
  verify "$NEW_HASH" && { write_status unchanged "$NEW_HASH"; exit 0; }
fi

run stellar contract install --wasm "$WASM" --source "$SOURCE" --network "$NETWORK"
if upgrade_to "$NEW_HASH" && verify "$NEW_HASH"; then
  write_status deployed "$NEW_HASH"
  exit 0
fi

echo "FAIL  deployment verification failed; rolling back to $PREVIOUS_HASH"
if upgrade_to "$PREVIOUS_HASH" && DRY_RUN_FAIL_VERIFY="" verify "$PREVIOUS_HASH"; then
  write_status rolled_back "$PREVIOUS_HASH"
  exit 1
fi
write_status rollback_failed "unknown"
exit 3
