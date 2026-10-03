# Contract Build Verification

Anyone can check that a deployed `ledgerlens-score` contract was built from a
specific commit of this repository. The process follows the Stellar ecosystem
build-verification convention (SEP-55) so explorers can display the result.

## What the release pipeline produces

For every `contract-v*` tag, `.github/workflows/release.yml`:

1. Builds the raw WASM with the pinned toolchain and optimizes it.
2. Writes `build-manifest.json` (repository, commit, tag, workflow ref,
   toolchain versions, raw and optimized SHA-256).
3. Creates a GitHub artifact attestation (`actions/attest-build-provenance`)
   for both WASM files. The attestation is a Sigstore-signed SLSA provenance
   statement binding the WASM digest to repository, commit, workflow file and
   the GitHub-hosted builder identity.
4. Publishes the optimized WASM, SHA-256 manifest and `build-manifest.json` on
   the GitHub release.

The contract also embeds `source_repo=github:Ledger-Lenz/Ledgerlens-contract`
in its `contractmetav0` custom section via `contractmeta!`. The value is
constant, so it does not affect reproducibility; explorers read it to locate
the attestation. The commit is deliberately not embedded (that would make the
WASM depend on build-time input); the attestation carries it instead.

Pre-release tags such as `contract-v1.4.0-rc.1` run the same pipeline and are
published as GitHub pre-releases. Test workflow changes on one of these tags
before cutting a final release.

## Verifying (auditors)

```bash
scripts/verify-build.sh <network> <contract-id> <release-tag>
```

The command fetches the deployed WASM, then checks:

- its hash equals `wasm.optimized_sha256` in the release manifest;
- `gh attestation verify` accepts it, signed by this repository's
  `release.yml`, for the commit the tag points to;
- a fresh local build at that commit reproduces the same hash (skip with
  `--no-rebuild`).

It exits non-zero if the hash differs (e.g. a different WASM was deployed) or
the commit differs (e.g. the tag was moved after release).

## Verifying (explorers)

1. Read `source_repo` from the contract's `contractmetav0` section.
2. Look up attestations for the deployed WASM hash:
   `GET /repos/{owner}/{repo}/attestations/sha256:{hash}`.
3. Verify the Sigstore bundle and show the commit and workflow from the
   certificate.

## Trust model and limits

- **Trusted:** GitHub Actions hosted runners, GitHub's OIDC issuer, Sigstore's
  Fulcio/Rekor, and the reviewed contents of `release.yml` at the tagged
  commit.
- **Proven:** the deployed bytes were produced by `release.yml` running on the
  stated commit. The local rebuild removes the need to trust the runner, as
  long as the build stays reproducible (see `docs/reproducible-builds.md`).
- **Not proven:** that the source is correct or safe, that the deployed
  instance is initialized or configured correctly, or anything about
  dependencies beyond `Cargo.lock` and the SBOM. A compromised maintainer able
  to push a tag can still produce a valid attestation for malicious code.
  Review the commit.
