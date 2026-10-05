# Upgrade Guide

This guide describes how to safely upgrade the LedgerLens contracts and how to
rehearse upgrades before they reach mainnet.

## Score contract

The score contract ships with an upgrade rehearsal harness in
`contracts/ledgerlens-score/src/upgrade_smoke.rs`. It builds fixtures of the
score storage from every released layout, upgrades them in place, and compares
observable behavior (scores, weights, pause state) against the expected values.

Run the rehearsal with:

```sh
cargo test -p ledgerlens-score upgrade_smoke
```

## Aggregator contract

The aggregator holds shard configuration and policies, so a bad upgrade can
silently mis-route consumer reads. The aggregator therefore mirrors the score
contract's rehearsal pattern in
`contracts/ledgerlens-aggregator/src/upgrade_smoke.rs`.

### What the harness covers

- **Storage fixtures for every released layout.** Each released aggregator
  storage layout has a fixture that is loaded into a fresh contract instance
  before the migration entry point runs.
- **Observable behavior comparison.** After upgrading in place, the harness
  compares the shard list, policies, quorum configuration, and pause states
  against the expected values. Any difference not listed in the changelog fails
  the test.
- **Migration entry-point rules.** The harness asserts that the migration entry
  point is idempotent (running it twice is a no-op), resumable (a partially
  applied migration can be continued), and atomic on failure (a failed migration
  leaves storage untouched).
- **Edge cases.** Upgrades are rehearsed while a shard is paused, while a
  configuration snapshot is pending, and with the maximum shard count.
- **Score interface compatibility.** For each supported version pair, the
  harness checks that the aggregator still satisfies the score contract's
  interface.

### Running the aggregator rehearsal

```sh
cargo test -p ledgerlens-aggregator upgrade_smoke
```

CI runs the aggregator upgrade matrix and fails on any observable behavior
difference that is not listed in the changelog.

### Rollback

Rollback fixtures exist for a failed migration. If a migration fails, the
harness verifies that the previous storage layout is still readable and that the
contract can be re-initialized from the rollback fixture.

## Compatibility policy

Any change to the public ABI, storage layout, events, or error enum must follow
the repository's compatibility policies and update the relevant docs and
generated artifacts. When adding a new storage layout, add a fixture to the
upgrade harness and document the observable behavior change in the changelog.
