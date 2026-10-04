# Security Policy

LedgerLens secures financial decisions, so we take vulnerability reports seriously and
want to make responsible disclosure easy, safe, and predictable for researchers.

## Reporting a vulnerability

**Please do not open a public issue for security problems.** Use one of the private
channels below:

1. **GitHub private vulnerability reporting (preferred).** Open the repository's
   **Security** tab and click **Report a vulnerability**. This creates a private
   advisory visible only to you and the maintainers.
2. **Email.** Send a report to `security@ledgerlens.example` (replace with the
   maintainer-operated mailbox before mainnet). Encrypt sensitive details with our
   PGP key.

### PGP / secure-form guidance

- PGP key fingerprint: `0000 0000 0000 0000 0000  0000 0000 0000 0000 0000`
  (publish the real key at `docs/security/pgp-key.asc` and in the repo before mainnet).
- If you cannot use PGP, use the GitHub private advisory form, which is encrypted in
transit and access-controlled.
- Never include private keys, seed phrases, or live credentials in a report. Use
  test vectors or redacted reproductions instead.

### What to include

- Affected component (contract, tool, script, or deployed address) and version/commit.
- A clear description of the impact and the conditions required to trigger it.
- A minimal reproduction (test, script, or transaction sequence) where possible.
- Your assessment of severity using the rubric below, and any suggested fix.

## Response time targets

| Stage | Target |
| --- | --- |
| Acknowledge receipt | within 2 business days |
| Initial triage & severity assignment | within 5 business days |
| Status update cadence | at least every 7 days until resolved |
| Fix or mitigation for Critical/High | within 30 days of triage |
| Fix or mitigation for Medium/Low | best effort, tracked in the advisory |

## Safe harbor

We will not pursue or support legal action against researchers who:

- Make a good-faith effort to follow this policy.
- Only interact with systems and accounts they own or are explicitly authorized to test.
- Avoid privacy violations, data destruction, and service degradation.
- Give us a reasonable time to fix the issue before any public disclosure.

If a third party initiates legal action against you for research conducted under this
policy, we will make it known that your actions were conducted in compliance with it.

## Disclosure timelines

- We follow **coordinated disclosure**. The default embargo is **90 days** from the
  acknowledged report, or sooner if a fix ships earlier.
- We may extend the embargo by mutual agreement when a fix requires a coordinated
  release (e.g. a contract migration or integrator notification).
- We will credit reporters in the published advisory unless they prefer to remain
  anonymous.
- If we cannot reach agreement, we ask that you notify us before disclosing so we can
  prepare integrators.

## Severity rubric

Severity is mapped to contract-specific impact and is kept consistent with
[`docs/incident-severity-classification.md`](docs/incident-severity-classification.md).

| Severity | Contract-specific impact | Examples | Reward tier |
| --- | --- | --- | --- |
| **Critical** | Direct, unconditional loss of funds for integrators; governance takeover; permanent protocol insolvency | Reentrancy draining balances; forged governance execution; storage corruption of core accounting | Tier 1 |
| **High** | Conditional fund loss; false-safe scores that mislead integrators into unsafe decisions; bypass of fail-closed gates | Score manipulation that reports safe when unsafe; auth bypass on privileged entrypoints | Tier 2 |
| **Medium** | Denial of service on core paths; bounded but material incorrectness; griefing with lasting state impact | Unbounded loop causing out-of-gas on a critical read; incorrect event/error semantics | Tier 3 |
| **Low** | Limited-impact issues, hardening gaps, or informational findings | Missing input validation with no exploitable path; documentation/ABI drift | Tier 4 / acknowledgment |

Reward amounts per tier are set by the maintainers and published alongside the bounty
program; tiers reflect impact, not effort. Duplicate reports are credited to the first
reporter.

## Scope

**In scope:**

- **Contracts:** all Soroban contracts in this repository, including their public ABI,
  storage layout, events, and error enum.
- **Tools:** build, deployment, and verification tooling maintained here.
- **Scripts:** migration, seeding, and operational scripts.
- **Deployed addresses:** the canonical mainnet/testnet deployments listed in
  `docs/security/deployed-addresses.md` (publish before mainnet).

**Out of scope:**

- Third-party dependencies and the Stellar/Soroban platform itself (report upstream).
- Issues requiring compromised maintainer credentials or physical access.
- Social engineering, phishing, and spam.
- Findings already documented as known limitations or accepted risks.
- Best-practice/style suggestions with no demonstrable security impact.
- Test-only code and mock contracts not deployed to production.

## Internal triage process

| Role | Owner | Responsibility |
| --- | --- | --- |
| Triage lead | Security maintainer (on-call) | Acknowledge, assign severity, drive the advisory |
| Contract owner | Core contract maintainer | Assess and prepare contract fixes |
| Tooling owner | Tooling maintainer | Assess and prepare tool/script fixes |
| Release owner | Release maintainer | Coordinate patched release and integrator notice |

### Private advisory workflow

1. **Intake.** Report arrives via private advisory or email; triage lead acknowledges
   within 2 business days and opens a **private GitHub security advisory**.
2. **Triage.** Triage lead assigns severity per the rubric and loops in the relevant
   owner. A dry-run of this workflow is recorded in `docs/security/dry-run.md`.
3. **Fix preparation.** Fixes are developed on a **private fork or private branch**
   referenced from the advisory — never in a public PR that reveals the issue.
4. **Review & test.** The fix is reviewed by at least one maintainer who did not author
   it, with regression tests added under the advisory.
5. **Release without early leakage.** The patched release is prepared privately and
   published together with the advisory. Public commits, changelog entries, and PRs
   are held until the coordinated disclosure date.
6. **Disclosure.** The advisory is published, reporters are credited, and integrators
   are notified through the documented channels.

## Contact

Security questions that are not vulnerabilities can be raised in the repository's
discussion channels. For everything sensitive, use the private channels above.
