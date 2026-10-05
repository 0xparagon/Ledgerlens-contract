# Dependency update policy

Automated updates are configured in [`renovate.json`](../renovate.json) (issue #1227).

## Grouping, pinning and scheduling

| Group | Packages | Schedule | Approval |
|---|---|---|---|
| `soroban-sdk` | `soroban-sdk`, `soroban-env-*`, `soroban-spec`, `soroban-sdk-macros`, `stellar-xdr` | Weekly (Mon, before 06:00 UTC) | Minor/patch: normal review. **Major: dependency-dashboard approval.** |
| `cryptography` | `sha2`, `sha3`, `*-dalek`, `k256`, `p256`, `blake2`, `subtle`, `rand*` | Weekly | **Dependency-dashboard approval** (linked into the WASM) |
| `github-actions` | All actions, pinned to digests | Monthly | Normal review |
| Security advisories | Any crate | Immediately | Labelled `security` |

- `rangeStrategy: pin`: all Cargo requirements are exact versions.
- PRs get the `wasm-linked` label when the crate ends up in the deployable `ledgerlens-score` WASM.
  Those PRs, and every SDK major, need a maintainer to tick them on the Dependency Dashboard issue before Renovate opens them.

### Dry run

```bash
npx --yes --package renovate -- renovate-config-validator --strict renovate.json
LOG_LEVEL=debug npx --yes renovate --platform=local --dry-run=full   # prints the grouped PRs it would open
```

The `renovate-config` job in [`sdk-compat.yml`](../.github/workflows/sdk-compat.yml) validates the config on every change.

## Compatibility job

[`sdk-compat.yml`](../.github/workflows/sdk-compat.yml) runs on any PR that touches a manifest, the lockfile or the toolchain.
It builds and tests two variants:

- **current**: the workspace as it is on the PR.
- **candidate**: the next SDK version (the `candidate` input, default `22.0.0`).

It then posts a PR comment comparing the WASM size, the test result (which includes the resource/CPU budget tests), and a diff of the exported contract functions (the ABI surface).
A failing candidate build is reported in the comment and does not block the PR.

## Reviewer checklist (SDK majors and WASM-linked crates)

- [ ] The compatibility comment is present and every ABI, size and budget delta is explained.
- [ ] [`cargo-deny`](../deny.toml) passes (the `dependency-license` / `audit` jobs in `ci.yml`).
- [ ] `cargo-vet` / the `supply-chain` job in `ci.yml` passes, or new audits/exemptions are added in this PR.
- [ ] The WASM size gate (`scripts/check-wasm-size.sh`) passes.
- [ ] For SDK majors: [`docs/soroban-sdk-migration.md`](soroban-sdk-migration.md) is updated with the impact analysis.
- [ ] [`docs/host-version-support-policy.md`](host-version-support-policy.md) and `rust-toolchain.toml` still agree.
- [ ] Storage layout, events and error enum are unchanged, or the change follows the compatibility policies.

## Advisory exception expiry

Advisories we cannot fix yet are ignored in `deny.toml` `[advisories].ignore`. Each entry must:

1. Use the table form `{ id = "RUSTSEC-YYYY-NNNN", reason = "... expires YYYY-MM-DD, owner @handle" }`.
2. Expire no more than **90 days** after it is added. When it expires, the owner either removes the ignore
   (after upgrading), or renews it with a fresh justification that a second maintainer approves.
3. Be reviewed at every release. `docs/release-process.md` releases must not ship with an expired exception.

Expired exceptions are treated as failing gates: reviewers must reject PRs that touch `deny.toml` while an expired entry is still present.
