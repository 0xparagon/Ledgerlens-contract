# Compliance and Governance Activity Report

A reproducible, evidence-grade summary of governance and operational activity
for a ledger range, generated from indexer data (issue #1222). Two parties
running the generator on the same corpus with the same arguments obtain
byte-identical `report.json` and `report.html` and the same content hash.

Generator: [`tools/compliance-report/generate.py`](../tools/compliance-report/generate.py)
(Python 3.9+, standard library only).

```bash
python3 tools/compliance-report/generate.py events.jsonl --out report/ \
  --ledger-from 1000 --ledger-to 3000 [--contract C... ...] \
  [--bucket-width 10] [--k 5] [--max-heartbeat-gap 720]
# prints the content hash; writes report/report.json and report/report.html
```

## Input: event corpus

JSON lines, one event per line. Every event has `type`, `ledger` (integer)
and `contract_id`; the optional `event_index` orders events within a ledger.
Line order does not matter.

| `type` | Fields | Source |
|---|---|---|
| `governance_action` | `proposal_id`, `parent_proposal_id?`, `action`, `actor`, `tx_hash?` | governance action registry / audit chain events ([audit-chain.md](audit-chain.md)) |
| `signer_change` | `signer`, `change` (`added` \| `removed` \| other) | signer governance events |
| `param_change` | `param`, `old?`, `new`, `proposal_id?` | parameter governance events |
| `heartbeat` | – | heartbeat events |
| `score` | `wallet`, `score` (integer 0–100) | score submission events |

Unknown event types are rejected rather than ignored, so a schema change
cannot silently drop evidence.

## Output: report data model

`report.json` (canonical JSON) contains:

| Field | Content |
|---|---|
| `report_version` | data model version (currently 1) |
| `scope` | `ledger_from`, `ledger_to`, `contract_ids` covered, `event_count` |
| `privacy` | redaction parameters applied (`min_bucket_size_k`, `score_bucket_width`) |
| `governance_actions` | each action with its **proposal lineage** (root → … → proposal, following `parent_proposal_id`) |
| `signer_set` | ordered signer changes with active-signer count after each, and final active set per contract |
| `parameters` | every parameter change with old/new value and originating proposal (**provenance**), plus current values |
| `heartbeat` | per contract: heartbeat count, **downtime intervals** (gaps longer than `--max-heartbeat-gap` ledgers, including the range edges) and total downtime ledgers |
| `score_distribution` | per contract **histogram** of scores, subject to the redaction policy |
| `content_hash` | SHA-256 over the canonical JSON of all fields above |

`report.html` renders the same data and displays the content hash.

## Determinism rules

* Events are sorted by `(ledger, event_index, contract_id, canonical JSON)`
  before processing, so corpus order is irrelevant.
* Output contains no timestamps, hostnames, paths, locale-dependent text or
  floating-point values.
* JSON is canonical: sorted keys, `,`/`:` separators, ASCII escapes, one
  trailing newline. `content_hash = sha256(canonical(report without content_hash))`.
* Verify a report you received:

  ```bash
  python3 -c "import json,hashlib,sys;r=json.load(open(sys.argv[1]));h=r.pop('content_hash');\
  print(h==hashlib.sha256(json.dumps(r,sort_keys=True,separators=(',',':')).encode()).hexdigest())" report.json
  ```

CI runs the golden-file tests on Linux and macOS (`python-tools` job); both
must reproduce the committed golden files byte-for-byte.

## Redaction policy

The report is intended for governance participants, integrators and auditors,
not for per-wallet analysis. The following policy applies:

1. **Wallet identifiers are never output.** `score` events contribute only to
   aggregate histograms; the `wallet` field is read and discarded.
2. **Individual scores are never output**, only bucket counts
   (`--bucket-width`, default 10 points; the last bucket includes 100).
3. **Small-count suppression (k-anonymity, default k = 5).** A histogram
   bucket with 1 to k−1 scores is published as `suppressed` with no count.
   Empty buckets are shown as 0, which reveals no individual data.
4. **Complementary suppression.** If exactly one bucket is suppressed its
   count would be recoverable from the total, so the smallest non-empty
   visible bucket is suppressed as well.
5. **Small totals.** If a contract has fewer than k scores in range, the
   total is also withheld (`total_scores: null`).
6. **Governance data is published in full**: actors, signers, proposal ids,
   parameter values and transaction hashes are already public on-chain and
   are the point of the report.
7. The report does not re-publish differentially private aggregate outputs'
   noise seeds or any input to the Laplace mechanism in
   [privacy-model.md](privacy-model.md).

## Privacy review

| Date | Reviewer | Scope | Outcome |
|---|---|---|---|
| 2026-09-27 | Report author (#1222), pending maintainer sign-off | Data model v1, redaction rules 1–7 | No wallet identifiers or individual scores reach the output (asserted by `test_no_wallet_identifiers_leak`); bucket suppression and complementary suppression verified by `test_small_buckets_suppressed`. Residual risk: an observer with an external list of a contract's scores in the range could still difference two reports over overlapping ranges; mitigated by generating reports only over agreed, non-overlapping periods. |

Changes to the data model or the redaction rules require a new row in this
table in the same PR.

## Tests

```bash
python3 -m unittest discover -s tools/compliance-report -p 'test_*.py' -v
# after an intentional output change:
UPDATE_GOLDEN=1 python3 -m unittest discover -s tools/compliance-report -p 'test_*.py'
```

The recorded corpus is `tools/compliance-report/testdata/corpus.jsonl`; the
golden outputs are in `testdata/golden/`.
