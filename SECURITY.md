# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability, please report it privately by emailing the maintainers rather than opening a public issue. We will acknowledge receipt within 72 hours and provide a timeline for a fix.

## Dependency Policy

This repository enforces a supply-chain policy for all Rust dependencies:

- **Advisory scanning** via `cargo-deny` (see `deny.toml`) rejects crates with known vulnerabilities.
- **License checks** via `cargo-deny` restrict dependencies to an approved license set.
- **Audit tracking** via [`cargo-vet`](https://mozilla.github.io/cargo-vet/) records which dependencies have been reviewed by a human and which are trusted via imported audit sets.

### Cargo Vet Criteria

We use two criteria to reflect how much trust a dependency needs:

| Criterion | Applies to | Meaning |
| --- | --- | --- |
| `safe-to-deploy` | Crates linked into the contract WASM | The crate has been reviewed and is considered safe to ship on-chain. This is the stricter criterion. |
| `safe-to-run` | Dev-only and tooling crates (tests, benches, CI helpers) | The crate is only executed locally or in CI and never linked into the deployed artifact. |

Crates that are linked into the WASM must meet `safe-to-deploy`; dev-only and tooling crates only need `safe-to-run`. This keeps the review burden focused on code that actually ships.

### Trusted Audit Imports

Rather than auditing every crate ourselves, we import audit sets from trusted organisations. The chosen sources are documented in `supply-chain/config.toml` under `[imports]`:

- `mozilla` — the Mozilla audit set, a widely used baseline for the Rust ecosystem.
- `bytecodealliance` — audits maintained by the Bytecode Alliance, relevant to our WASM toolchain.
- `google` — the Google audit set, covering many common crates.

Imports are pinned to a specific revision so that upstream changes are reviewed before they take effect.

### Adding or Renewing an Audit

When CI reports an unaudited dependency, follow this workflow:

1. Run `cargo vet` locally to see the missing audits.
2. Prefer importing an existing audit from a trusted source:
   ```
   cargo vet certify <crate> <version>
   ```
   or add the crate to an existing import if a trusted set already covers it.
3. If no trusted audit exists, perform a manual review of the crate and record it:
   ```
   cargo vet certify <crate> <version> --criteria safe-to-deploy
   ```
   Use `safe-to-run` instead for dev-only or tooling crates.
4. If a review cannot be completed before the dependency is needed, add a time-boxed exemption in `supply-chain/config.toml` with a rationale and a review-by date, and open a tracking issue.
5. Commit the updated `supply-chain/` files alongside the dependency change.

### Exemptions

Exemptions are a last resort. Every exemption must record:

- the crate and version,
- the criterion it is exempt from,
- a written rationale explaining why the audit is deferred,
- an owner responsible for completing the review, and
- a review-by date after which the exemption must be renewed or removed.

The exemption list is kept as small as possible; CI fails when a new unaudited dependency is added to a contract crate, so exemptions cannot silently accumulate.

### CI Gate

The `supply-chain` CI job runs `cargo vet` on every pull request. It fails when a dependency linked into a contract crate is neither audited nor covered by an imported audit set, ensuring new unaudited dependencies cannot be merged without review.
