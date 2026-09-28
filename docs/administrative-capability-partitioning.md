# Administrative Capability Partitioning

Issue: #695 — Add administrative capability partitioning by operation risk.

## The problem this closes

Before this change, every privileged endpoint in `ledgerlens-score` —
pausing the contract, changing score-gating parameters, proposing a WASM
upgrade, and managing the admin signer set itself — was gated by exactly
the same check: `require_admin_auth`, backed by one shared admin
key/M-of-N set. Whoever could satisfy that one quorum could do all of it.
A single compromised admin signer set (or a colluding quorum) could pause
the contract, rewrite score thresholds, push a WASM upgrade, *and* rotate
the admin set itself, with no way to require a different, disjoint set of
approvers for the more dangerous operations.

## The fix

`Policy` (in `types.rs`) names five administrative capabilities,
partitioned by operation risk:

| Policy | Representative endpoints |
|---|---|
| `ScorePolicy` | `propose_parameter_change`, `execute_parameter_change` |
| `UpgradeGovernance` | `propose_upgrade`, `execute_upgrade`, `veto_upgrade` |
| `EmergencyPause` | `pause`, `unpause` |
| `DataDeletion` | `clear_score`, `clear_score_history` (pre-existing — see below) |
| `SignerAdmin` | `add_admin_signer`, `remove_admin_signer`, `set_admin_threshold` |

Each mapped endpoint still requires routine admin quorum via
`require_admin_auth` first (that check is unchanged). On top of that,
`Self::require_policy_auth(env, policy, admin_signers)` looks up an
optional, independently configured `PolicyApproval { enabled, approver }`
for that policy and, when `enabled`, additionally requires
`approver.require_auth()`. The approver must stay disjoint from the
routine admin key/set — checked by `set_policy_approval`, which rejects an
overlapping or missing approver (fail-closed: an enabled policy can never
end up silently equivalent to "no extra check").

Because each policy's approver is configured and checked independently,
authorization obtained under one policy does not carry over to another:
the wrong approver's address simply never satisfies `require_auth()` for
the endpoint it wasn't configured for. See
`test_cross_policy_approver_reuse_fails` in
`test_capability_partitioning.rs`, which configures two *real*, currently
enabled approvers for two different policies and proves the `SignerAdmin`
approver cannot authorize a `EmergencyPause`-gated call.

### `Policy::DataDeletion` is a pointer, not new code

`DataDeletion` reuses the pre-existing `DeletionApprovalPolicy` /
`require_deletion_auth` / `set_deletion_approval_policy` mechanism that
already gated `clear_score` and `clear_score_history` before this issue.
It is listed in the `Policy` enum purely so all five categories named in
#695 share one canonical, documented identifier. `set_policy_approval`
explicitly rejects `Policy::DataDeletion` with `Error::InvalidPolicy` so
there remains exactly one configuration entry point per policy — operators
configure deletion via `set_deletion_approval_policy`, and the other four
via `set_policy_approval`.

## Bounded resource use

`require_policy_auth` does one instance-storage read (`get_policy_approval`)
and, when enabled, exactly one `require_auth()` call — the same constant
amount of work regardless of which policy or how many times it's called.
No unbounded loop or caller-controlled iteration is introduced.

## Compatibility summary

- **No new `Error` variant** — the `#[contracterror]` enum is already at
  the 50-variant XDR cap; `Error::InvalidPolicy` and `Error::InvalidThreshold`
  (both existing aliases/discriminants) are reused.
- **No change to existing storage keys** — `PolicyApprovalEnabled(Policy)`
  / `PolicyApprovalApprover(Policy)` are new `DataKeyD` variants; the
  pre-existing `DeletionPolicyEnabled` / `DeletionApprover` keys and their
  semantics are untouched.
- **New event `pol_appr`** (topics: `pol_appr`, version, `policy`; data:
  `(enabled, approver)`) — additive, emitted only by the new
  `set_policy_approval`. Existing events are unaffected.
- **New public ABI surface**: `set_policy_approval`, `get_policy_approval`,
  and the `Policy` / `PolicyApproval` types, all additive.
- **Behavior change (only when explicitly configured):** by default
  (`enabled = false` for every policy), `pause`, `unpause`,
  `propose_upgrade`, `execute_upgrade`, `veto_upgrade`,
  `propose_parameter_change`, `execute_parameter_change`,
  `add_admin_signer`, `remove_admin_signer`, and `set_admin_threshold`
  behave exactly as before — `require_policy_auth` falls through to the
  unchanged `require_admin_auth` result. Only after an operator opts a
  policy in via `set_policy_approval` does that policy's mapped endpoints
  require the extra approver signature.

## Feature-flag registry with timelocked kill switches (#1151)

Issue #1151 adds a second, orthogonal administrative axis: a governed
feature-flag registry that can *disable* an individual optional subsystem
quickly, while *re-enabling* it stays subject to a timelock and a higher
quorum. This is deliberately separate from the capability partitioning
above (which governs *who* may call an endpoint) and from the global and
pair pauses (which halt *all* or *paired* activity).

### Flag set and meaning of "disabled"

`FeatureFlag` (in `types.rs`) names one stable identifier per optional
subsystem. Each flag documents what "disabled" means for its guarded
entry points:

| Flag | Subsystem | Disabled means |
|---|---|---|
| `ZkRangeProofs` | zero-knowledge range proofs | reject writes **and** reads |
| `VerkleCommitments` | Verkle commitments | reject writes **and** reads |
| `Delegation` | delegation | reject writes (reads still served) |
| `Disputes` | disputes | reject writes (reads still served) |
| `Escrow` | escrow | reject writes **and** reads |
| `OracleAdapter` | oracle adapter | reject writes (reads still served) |

"Reject writes" means any state-mutating entry point of that subsystem
returns `Error::FeatureDisabled`; "reject reads" means its read-only
entry points return the same error. Flags default to *enabled* (the
subsystem is live), so behavior is unchanged until an operator acts.

### Fast disable, timelocked enable

- **Disable** is fast and guardian-level: `disable_feature(env, flag)`
  requires `require_guardian_auth` and takes effect immediately.
- **Enable** is slow and higher-quorum: `propose_enable_feature(env, flag)`
  records a pending enable with `now + FEATURE_ENABLE_TIMELOCK`, and
  `execute_enable_feature(env, flag)` only succeeds after the timelock has
  elapsed and requires the higher `require_admin_auth` quorum.
- Both directions emit events: `feat_dis` (disable) and `feat_en`
  (enable executed), plus `feat_en_prop` for the proposal.

### One shared check helper

Every guarded entry point calls the single helper
`Self::require_feature_enabled(env, flag)`, which reads the flag and
returns `Error::FeatureDisabled` when it is off. No entry point inlines
its own flag read, so the check is uniform and auditable.

### Structural test

`test_feature_flag_guard_coverage` in `test_feature_flags.rs` enumerates
the public entry points of each flagged module and asserts each one calls
`require_feature_enabled` for its flag. A new public entry point added to
a flagged module without the guard fails this test.

### Interaction with global pause and pair pause

These three mechanisms are independent and compose as follows:

| Mechanism | Scope | Effect |
|---|---|---|
| Global pause | whole contract | all entry points reject |
| Pair pause | a specific pair | that pair's entry points reject |
| Feature flag | one optional subsystem | that subsystem's entry points reject |

A call is permitted only when the global pause is off, the relevant pair
is not paused, **and** the subsystem's feature flag is enabled. The
feature flag is checked *after* the pause checks, so a globally paused
contract still reports the pause error first. Disabling a feature never
bypasses a pause, and pausing never bypasses a disabled feature.

### `supports_interface`

Flags are published through `supports_interface` so consumers can detect
which optional subsystems are currently live and degrade gracefully
instead of calling into a disabled subsystem.

### Compatibility summary

- **New `Error::FeatureDisabled`** variant (additive; within the XDR cap).
- **New storage keys** `FeatureEnabled(FeatureFlag)` and
  `FeatureEnablePending(FeatureFlag)` — additive, no existing key changes.
- **New events** `feat_dis`, `feat_en_prop`, `feat_en` — additive.
- **New public ABI surface**: `disable_feature`, `propose_enable_feature`,
  `execute_enable_feature`, `is_feature_enabled`, and the `FeatureFlag`
  type, all additive.
- **Behavior change (only when explicitly configured):** every flag
  defaults to enabled, so all guarded entry points behave exactly as
  before until an operator disables a feature.
