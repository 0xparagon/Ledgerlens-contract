# Submission and discussion material

**Issue:** [#1248 — Draft a standard for an on-chain risk-score registry interface](https://github.com/Ledger-Lenz/Ledgerlens-contract/issues/1248)
**Artefact:** [`docs/standards/sep-risk-score-registry-interface.md`](sep-risk-score-registry-interface.md) v0.1.0 (Draft)

This file is the working material for taking the draft into ecosystem review: what
is being proposed, where it has been posted, what has come back, and what a
maintainer still has to do by hand. It is a checklist with a paper trail, not
marketing.

---

## 1. Proposal summary (for a forum post)

> **Draft: a minimal on-chain risk score registry interface.**
>
> Risk scores are only useful if protocols can read them, and today that means
> binding to one registry's ABI. This draft fixes the smallest surface that
> makes providers swappable: an infallible, fail-closed `query_risk_gate`, a
> confidence-gated variant, an optional score payload, and capability discovery.
>
> What it deliberately leaves out: how scores are produced, who may produce
> them, and how a provider is governed. That is what makes it provider-neutral.
>
> It ships with three things that make it falsifiable rather than aspirational:
> a reference provider written against the *existing* LedgerLens surface, a
> data-only conformance suite (33 vectors, any language) that the shipped
> `ledgerlens-score` already passes with no ABI change, and a clause-by-clause
> alignment document recording every divergence and its reason.
>
> The clause I would most like feedback on is **9.1/9.2** (what a read is allowed
> to write) and **5.6** (why freshness is the consumer's job rather than the
> provider's). The clause I am least sure about is **3.3** — whether
> `get_score` should be SHOULD at all.

## 2. Where it has been posted

| Venue | Status | Link |
|---|---|---|
| Ledgerlens-contract issue #1248 (design note) | posted | see issue timeline |
| Ledgerlens-contract issue #1248 (PR) | pending | — |
| Stellar Discord `#soroban-contracts` / `#standards` | **not posted — maintainer action required** | — |
| Stellar Developer Forum | **not posted — maintainer action required** | — |
| SEP pull request against `stellar/stellarsep` (once assigned a number) | **blocked on SEP number assignment** | — |

### Why the ecosystem-channel row is still open

Acceptance criterion 2 asks that the draft "is discussed in the ecosystem's
public channel with recorded feedback". That step is not available from this
repository: posting to Stellar's Discord or developer forum requires an
account on those platforms, which cannot be created or used from a code
contribution. The announcement text is §1 above, ready to paste; the feedback
record is §4 below. **A maintainer with those accounts needs to post §1 and then
paste the resulting thread link here**, and criterion 2 is not met until that
happens. Nothing in this document should be read as claiming it has been met.

## 3. Design note (posted on the issue before the PR)

The design note requested by #1248 covered:

- **Approach.** A provider-neutral subset of the existing `ILedgerLensScore`
  surface, written in SEP format, with a reference provider and a data-only
  conformance suite. The portable core is three reads plus discovery.
- **Storage/ABI impact on the shipped contract: none.** No function signature,
  storage key, event, or error discriminant of `contracts/ledgerlens-score`
  changes. Two of its rustdoc comments and parts of `docs/interface-spec.md`
  are corrected for accuracy, because the draft's §9.1 exposed an overclaim
  ("side-effect free" where the code writes a temporary marker).
- **The one deviation.** `ledgerlens-score` does not advertise the draft's `risk`
  root capability. It is SHOULD-level, deliberately not fixed in this PR, and
  asserted in a test so the deviation list cannot silently grow or shrink.
- **Test plan.** The conformance suite runs against both the reference provider
  and the shipped contract; the harness carries a deliberately non-conformant
  provider as a negative control, because a suite that cannot fail is worse than
  no suite.

## 4. Feedback record

One row per substantive comment, with what changed as a result. "No change" is a
legitimate outcome and is recorded as such — a review that produced agreement
should be visible too.

| # | Date | Venue | Reviewer | Comment | Disposition |
|---|---|---|---|---|---|
| 1 | 2026-09-26 | issue #1248 | — | Design note posted; no maintainer response yet | open |

## 5. Open questions for reviewers

Deliberately unresolved in v0.1.0, listed so that disagreement has somewhere to
land:

1. **Is SHOULD the right level for `get_score`?** A gate-only provider is
   legitimate, but it also means a provider can be conformant while offering a
   consumer no way to classify a denial or enforce freshness. Alternative:
   require the payload, and accept that some providers must invent fields they
   have no honest answer for.
2. **Should a `risk` root capability exist at all?** It is the only clause that
   would have required a change to the shipped contract. The alternative is to
   drop it and rely on `cgate`/`score` presence as the discovery signal.
3. **Is a 7-day reference freshness bound in the vectors too opinionated?**
   `CONS-004` uses 604800s as a worked example. It is arbitrary, and a reviewer
   may reasonably say the suite should not imply a number.
4. **Should the suite gate on `requires_capability` at all?** It is what lets a
   lean provider be conformant, but it also means a provider can make a vector
   disappear by not claiming the capability that would trigger it.
5. **Should `subject` be typed as something other than `Address`?** An `Address`
   can be an account, a contract, or a muxed account. A consumer that assumes
   "wallet" may be wrong.

## 6. Maintaining this file

- Add a row to §4 for every substantive comment, including your own reply.
- Update §2 when a venue is actually posted to. Do not mark a venue posted
  because text is prepared.
- When v0.2.0 is cut, move §4 into a changelog entry in the SEP and keep only
  the open items here.
