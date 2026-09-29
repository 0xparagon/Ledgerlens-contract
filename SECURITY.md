# Security Policy

## Scope

This policy covers the **`ledgerlens-score` Soroban smart contract** and the surrounding deployment tooling in this repository.

Out-of-scope:
- The off-chain detection pipeline (`core`, `data` repos)
- The public API server (`api` repo)
- The web dashboard (`dashboard` repo)

## Supported Versions

| Contract version | Status  |
|-----------------|---------|
| 1.x (testnet)   | Active  |
| 0.x (pre-release)| Not supported |

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Report security issues by emailing **security@ledgerlens.io** with the subject line:

```
[SECURITY] <short description>
```

Include:

1. A clear description of the vulnerability and the affected contract function(s).
2. Steps to reproduce or a proof-of-concept (PoC) — even a pseudocode sketch helps.
3. The potential impact (e.g. unauthorized score submission, admin key extraction, fund loss if integrated with an AMM).
4. Your contact details if you would like to be credited.

## Response Timeline

| Milestone                     | Target            |
|------------------------------|-------------------|
| Acknowledgement              | Within 48 hours   |
| Triage and severity rating   | Within 7 days     |
| Fix or mitigation in testnet | Within 21 days    |
| Public disclosure            | After fix ships   |

We follow [Responsible Disclosure](https://en.wikipedia.org/wiki/Coordinated_vulnerability_disclosure). We will not take legal action against researchers who follow this policy.

## Secret Scanning & Pre-Commit Guardrails

Operator scripts under `scripts/` and `tools/recovery` handle Stellar secret
keys, RPC credentials and network passphrases. A committed secret in a public
repository is an incident, so secret scanning is layered: a local pre-commit
hook, a per-PR CI scan of the diff, and a scheduled CI scan of the entire
history.

### Scanner configuration

Scanning is driven by [gitleaks](https://github.com/gitleaks/gitleaks) using
`.gitleaks.toml`. The config extends the default ruleset and adds rules for:

- **Stellar secret keys** — `S` followed by 55 base32 characters (`S[A-Z2-7]{55}`).
- **Common API token formats** — GitHub (`ghp_`, `gho_`, `ghs_`, `ghr_`,
  `github_pat_`), Slack (`xox[baprs]-`), AWS access key IDs (`AKIA`/`ASIA`),
  and generic `api_key`/`token`/`secret` assignments.
- **Private key blocks** — PEM `-----BEGIN ... PRIVATE KEY-----` headers.

Test fixtures and documentation that intentionally contain fake, non-functional
values are excluded via `[allowlist]` paths (`tests/`, `fixtures/`, `*.md`,
`*.example`) and a regex allowlist for obvious placeholders (`EXAMPLE`,
`REDACTED`, `CHANGEME`, `xxxx`). This keeps the scan tuned to avoid false
positives on benign fixtures while still failing on real material.

### Running the scan locally

Install the pre-commit hook with a single command:

```sh
./scripts/install-hooks.sh
```

This installs a `pre-commit` hook that runs `gitleaks protect --staged` before
every commit and blocks the commit if a secret is detected. To scan the working
tree manually:

```sh
gitleaks detect --source . --config .gitleaks.toml
```

### CI enforcement

- **Pull requests** — the `secret-scan` job runs `gitleaks detect` against the
  PR diff. A seeded fake secret in a test branch fails the job and blocks merge.
- **Scheduled** — a nightly `secret-scan-history` job runs
  `gitleaks detect --log-opts=--all` over the full history. It reports cleanly
  on the current history or produces a triaged baseline (see below).

### Baseline handling

Existing benign matches are recorded explicitly in `.gitleaksignore` (one
fingerprint per line). The scheduled history scan consults this baseline so
that known, reviewed matches do not fail the build, while any *new* match does.
Adding a fingerprint to the baseline is a reviewed change: it must be justified
in the PR description and, where relevant, linked to the fixture that produced
it.

### Remediation playbook

If a secret is committed — or the scanner flags one — follow these steps in
order. **Revoke first; assume the secret is already compromised.**

1. **Revoke** — immediately disable or delete the exposed credential at its
   source (rotate the Stellar keypair, revoke the API token, remove the RPC
   credential). Do not wait for the purge to complete.
2. **Rotate** — issue a replacement credential and update it in the secret
   store / CI environment. Never re-use the exposed value.
3. **Purge** — remove the secret from the working tree and rewrite history so
   the value is no longer reachable (e.g. `git filter-repo` or the BFG). Force-
   push the rewritten branch and coordinate with anyone who has a clone. Add a
   fingerprint to `.gitleaksignore` only for benign, non-secret matches.
4. **Notify** — inform the security contact (`security@ledgerlens.io`) and any
   affected integrators. Record the incident, the rotation, and the purge in the
   security log. If the secret granted on-chain privileges, follow the upgrade
   governance flow above to rotate the admin/service key.

## Contract Threat Model

| Attack vector                        | Mitigation                                                        |
|--------------------------------------|-------------------------------------------------------------------|
| Deployment initialization front-running | Both contract initializers require authorization from the nominated admin before writing any privileged state |
| Unauthorized score write             | `submit_score` requires `service.require_auth()`                  |
| Compromised service key              | `pause()` halts submissions; `set_service()` rotates the key      |
| Accidental admin key loss            | Two-step transfer: new admin must call `accept_admin()`           |
| Score poisoning via out-of-range data | `score` and `confidence` clamped to 0-100 on-chain               |
| Resource exhaustion via oversized symbols or cryptographic byte payloads | `asset_pair` symbols are capped at 9 bytes and malformed oversized commitment / dispute replay byte payloads reject before hashing, proof parsing, shard fan-out, or storage writes |
| DoS via unbounded storage            | History ring buffer capped at `HISTORY_MAX_DEPTH` (10) per pair  |
| Large batch denial of service        | Batch size capped at `MAX_BATCH_SIZE` (20) per invocation        |
| M-of-N `signers`/`admin_signers` Vec padding | Signer lists bounded by the current signer-set size before any per-signer storage read or `require_auth` call (`TooManySigners`) — see "Memory-Exhaustion & Nested Input Bounds" below |
| Compromised service floods a pair with submissions | Per-`(wallet, asset_pair)` cooldown (`RateLimitExceeded`); admin-bounded `[MIN_COOLDOWN_SECS, MAX_COOLDOWN_SECS]`, with `override_rate_limit` as an audited emergency escape hatch |
| Silent malicious contract upgrade    | Time-locked upgrade governance (see below): mandatory delay + on-chain proposal anyone can inspect, plus admin veto |
| Data-residency / GDPR erasure request | `clear_score_history` and `clear_score` (admin-only) permanently remove scoring data from persistent storage; `clr_hist` / `clr_scr` events provide an on-chain audit trail of every erasure |

The complete caller, signer, and contract-as-caller review is recorded in
[`docs/cross-contract-authorization-audit.md`](docs/cross-contract-authorization-audit.md).
The corresponding lifecycle and adversarial coverage index is
[`docs/critical-state-transition-matrix.md`](docs/critical-state-transition-matrix.md).

## Upgrade Governance & Threat Model

Soroban contracts are immutable once deployed, but the admin can replace the
entire WASM via `env.deployer().update_current_contract_wasm(...)`. Left
ungoverned, a single admin key (or a compromised one) could swap in a backdoor
— disabling auth checks, redirecting score writes, or bricking integrations —
in **one transaction, with no warning**. To remove that single point of
instant failure, upgrades are gated behind an on-chain time-lock.

### The flow

1. **Propose** — the admin calls `propose_upgrade(new_wasm_hash)`. This stores
   an `UpgradeProposal` (committed hash, `proposed_at`, `executable_after`,
   `proposed_by`) and emits `upgrade_proposed`. It does **not** change the code.
2. **Monitoring window** — for at least `MIN_UPGRADE_DELAY_SECS` (48 hours;
   configurable up to 14 days) nothing can execute. Anyone — users, monitoring
   bots, integrating protocols — can call `get_pending_upgrade` to read the
   committed hash and `executable_after`, diff the proposed WASM, and alert the
   community.
3. **Execute or veto** — only after `executable_after` can the admin call
   `execute_upgrade`, which re-checks the clock at execution time (never a
   cached decision) before installing the WASM. At any point during the window
   the admin can `veto_upgrade` to cancel — the escape hatch if a proposal is
   malicious or the key was compromised. The veto emits `upgrade_vetoed` naming
   the caller, completing the audit trail.

### Threat model

| Concern | Mitigation |
|---------|------------|
| Admin pushes a backdoor instantly | No instant path exists — every upgrade waits out the full delay before `execute_upgrade` will run |
| Compromised **service** key triggers an upgrade | Service keys have no upgrade powers; only the current admin can propose/execute/veto |
| Caller manipulates the time-lock | Deadlines derive from `env.ledger().timestamp()`, which is deterministic and not caller-settable |
| Stale/early execution | `execute_upgrade` re-verifies `now >= executable_after` on every call |
| Admin shortens the window to rush an upgrade | `set_upgrade_delay` is bounded to `[MIN, MAX]`; it can never go below 48 h, and a lowered delay only applies to *future* proposals — an in-flight proposal keeps its original `executable_after` |
| No record of who acted | `UpgradeProposal.proposed_by` plus the `upgrade_*` events give a full on-chain audit trail |

**Safe vs. sensitive delay changes:** *raising* `MIN`-bounded delay is always
safe (it only lengthens scrutiny). *Lowering* the configured delay shortens the
community veto window and should only be done with broad community consensus.

### What monitors should watch

Subscribe to the `upgrade_proposed` event (or poll `get_pending_upgrade`). On a
new proposal, verify the committed `new_wasm_hash` against a reviewed,
reproducible build before `executable_after`. An unexpected proposal — or one
whose hash does not match a published, audited build — is the signal to raise
an alarm and, if warranted, push for a `veto_upgrade`.

## Memory-Exhaustion & Nested Input Bounds (#612)

### Assets and actors

- **Asset:** the contract's CPU-instruction and memory budget for a single
  invocation, shared by every caller in that ledger close. A single
  over-large call can burn a disproportionate share of it before failing.
- **Actors:** any address able to invoke a public entry point — including
  a **contract-as-caller** in a composability setup (e.g. an aggregator or
  gateway forwarding a batch on a user's behalf), which is no more trusted
  than a direct EOA caller for sizing purposes.
- **Trust assumption:** the `signers` / `admin_signers` / `submissions` /
  `proof` arguments are entirely attacker-controlled in shape and size,
  even when the *content* (a valid address, a valid signature) requires a
  real credential the attacker may not have.

### Nested shapes in scope

`submit_scores_bat

/* … truncated 4303 chars — edit only what you need near the top … */
