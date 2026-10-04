#!/usr/bin/env bash
# Verify that a deployed ledgerlens-score contract was built from a specific
# release of this repository (SEP-55 build-verification convention).
#
# Usage:
#   ./scripts/verify-build.sh <network> <contract-id> <release-tag> [--no-rebuild]
#
# Checks, in order:
#   1. Fetches the deployed WASM and hashes it.
#   2. Downloads the release's build-manifest.json and compares the hash.
#   3. Verifies the GitHub artifact attestation for the deployed WASM and that
#      it was produced from the commit the tag points to, by release.yml.
#   4. (default) Rebuilds from the tag locally and compares hashes.
#
# Requires: stellar, gh (authenticated), jq, sha256sum, git, cargo.
#
# Exit codes: 0 verified, 1 verification failed, 2 usage error.

set -euo pipefail

REPO="${VERIFY_REPO:-Ledger-Lenz/Ledgerlens-contract}"
REBUILD=true
POSITIONAL=()
for arg in "$@"; do
  case "$arg" in
    --no-rebuild) REBUILD=false ;;
    --help) sed -n '2,18p' "$0"; exit 0 ;;
    *) POSITIONAL+=("$arg") ;;
  esac
done
[ "${#POSITIONAL[@]}" -eq 3 ] || { sed -n '5,6p' "$0" >&2; exit 2; }
NETWORK="${POSITIONAL[0]}"; CONTRACT_ID="${POSITIONAL[1]}"; TAG="${POSITIONAL[2]}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
fail() { echo "FAIL  $*" >&2; exit 1; }

# 1. Deployed hash
stellar contract fetch --id "$CONTRACT_ID" --network "$NETWORK" --out-file "$WORK/deployed.wasm"
DEPLOYED_SHA="$(sha256sum "$WORK/deployed.wasm" | awk '{print $1}')"
echo "INFO  deployed WASM sha256: $DEPLOYED_SHA"

# 2. Release manifest
gh release download "$TAG" --repo "$REPO" --pattern build-manifest.json --dir "$WORK"
MANIFEST_SHA="$(jq -r .wasm.optimized_sha256 "$WORK/build-manifest.json")"
MANIFEST_COMMIT="$(jq -r .commit "$WORK/build-manifest.json")"
[ "$DEPLOYED_SHA" = "$MANIFEST_SHA" ] || fail "deployed hash != release manifest ($MANIFEST_SHA)"
echo "PASS  deployed hash matches release manifest"

# 3. Attestation: signed by release.yml in $REPO, for the tagged commit
gh attestation verify "$WORK/deployed.wasm" --repo "$REPO" \
  --signer-workflow "$REPO/.github/workflows/release.yml" \
  --format json > "$WORK/attestation.json" || fail "no valid attestation for deployed WASM"
ATTESTED_COMMIT="$(jq -r '.[0].verificationResult.signature.certificate.sourceRepositoryDigest' "$WORK/attestation.json")"
TAG_COMMIT="$(gh api "repos/$REPO/commits/$TAG" --jq .sha)"
[ "$ATTESTED_COMMIT" = "$TAG_COMMIT" ] || fail "attested commit $ATTESTED_COMMIT != $TAG commit $TAG_COMMIT"
[ "$ATTESTED_COMMIT" = "$MANIFEST_COMMIT" ] || fail "attested commit != manifest commit $MANIFEST_COMMIT"
echo "PASS  attestation verified (commit $ATTESTED_COMMIT)"

# 4. Independent local reproducible build
if [ "$REBUILD" = true ]; then
  git clone --quiet "https://github.com/$REPO" "$WORK/src"
  git -C "$WORK/src" checkout --quiet "$TAG_COMMIT"
  (cd "$WORK/src" &&
    cargo build --target wasm32-unknown-unknown --release -p ledgerlens-score --locked &&
    stellar contract optimize --wasm target/wasm32-unknown-unknown/release/ledgerlens_score.wasm \
      --output "$WORK/local.wasm")
  LOCAL_SHA="$(sha256sum "$WORK/local.wasm" | awk '{print $1}')"
  [ "$LOCAL_SHA" = "$DEPLOYED_SHA" ] || fail "local rebuild $LOCAL_SHA != deployed $DEPLOYED_SHA"
  echo "PASS  local reproducible build matches"
fi

echo "OK    $CONTRACT_ID on $NETWORK is verified as $TAG ($TAG_COMMIT)"
