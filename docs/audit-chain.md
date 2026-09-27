# Admin Governance Audit Chain

**Status:** Proposed · **Contract:** `LedgerLensScoreContract` · introduced in `CONTRACT_VERSION` 4.

The contract maintains a cryptographically verifiable audit trail of all admin governance actions via a Merkle chain. This allows off-chain operators to audit the contract's governance history without replaying every action, and provides on-chain evidence of what state changes have been authorized.

## 1. Action Hash Schema

For each admin action, a single canonical hash is computed:

```
action_hash = sha256(action_name || actor || params || timestamp)

Where:
- `action_name`: 8-byte Symbol (e.g., `symbol_short!("pause")`, `symbol_short!("upg_prop")`)
- `actor`: Address (the admin who performed the action), serialized via `to_xdr()`
- `params`: Variable-length parameter bytes specific to the action (e.g., new_wasm_hash for upgrades)
- `timestamp`: u64 ledger timestamp, little-endian (8 bytes)

## 2. Merkle Chain Formula

The audit root is updated after each action:

```
new_root = sha256(old_root || action_hash)

- Start with genesis root = `sha256([0; 32])` (all zeros) at initialization
- Each subsequent action appends to the chain

## 3. Reading the Chain

`get_admin_audit_root()` returns the current root.

To verify off-chain:
1. Fetch all admin action events from the blockchain (events emit the timestamp, actor, action name, and action-specific data)
2. Reconstruct action_hash for each event using the schema above
3. Replay the chain: `root_0 = genesis; root_i = sha256(root_{i-1} || action_hash_i)`
4. Compare the final root with the on-chain `get_admin_audit_root()`

If the roots match, the governance history is authentic and unaltered.

## 4. Genesis Root

At initialization, the audit root is set to:

```
genesis_root = sha256(0x0000000000000000000000000000000000000000000000000000000000000000)

(SHA-256 of 32 zero bytes)

## 5. Tracked Actions

All of the following admin state-changing functions emit audit events:

- `pause`
- `unpause`
- `propose_upgrade`
- `execute_upgrade`
- `transfer_admin`
- `accept_admin`
- `cancel_admin_transfer`
- `add_service_signer`
- `remove_service_signer`
- `set_service_threshold`
- `set_signer_rotation_ttl`
- `set_signer_rotation_grace`
- `set_service_pubkey`
- `set_aggregate_service_pubkey`
- `set_consensus_config`
- `set_reveal_window`
- `set_pair_paused`
- `veto_upgrade`
- `set_upgrade_delay`
- `set_watchlist`
- `set_escalation_threshold`
- `set_risk_threshold`
- `set_jump_threshold`
- `set_hysteresis_margin`
- `set_score_embargo`
- `revoke_all_embargoes`
- `resolve_dispute_admin`
- `set_staleness_window`
- `set_decay_rate`
- `set_cooldown`
- `set_pair_cooldown`
- `set_score_velocity_cap`
- `set_finality_buffer`
- `set_history_max_depth`
- `set_global_min_confidence`
- `set_score_delegate`
- `set_pair_weight`
- `set_pair_weight_batch`

And others. Consult the contract source for the complete list.

## 6. Off-Chain Verification Example

```python
import hashlib

def verify_audit_chain(events, on_chain_root_bytes):
    """
    Verify an audit chain by replaying events.
    
    events: list of dicts with keys:
      - action_name: str (e.g., 'pause')
      - actor_xdr: bytes (Address serialized to XDR)
      - params_bytes: bytes (action-specific parameters)
      - timestamp: int (ledger timestamp)
    on_chain_root_bytes: bytes (32-byte root from get_admin_audit_root())
    """
    root = hashlib.sha256(bytes(32)).digest()
    
    for event in events:
        action_bytes = event['action_name'].encode('ascii')
        # Pad to 8 bytes
        action_bytes = action_bytes.ljust(8, b'\0')[:8]
        
        action_preimage = (
            action_bytes +
            event['actor_xdr'] +
            event['params_bytes'] +
            event['timestamp'].to_bytes(8, 'little')
        )
        action_hash = hashlib.sha256(action_preimage).digest()
        
        chain_preimage = root + action_hash
        root = hashlib.sha256(chain_preimage).digest()
    
    return root == on_chain_root_bytes
```yaml

## 7. Replay Evidence Bundles

The replay harness in [tools/replay/src/main.rs](tools/replay/src/main.rs) can emit an incident evidence bundle that packages the replayed transactions, generated events, a configuration snapshot, issue references, and SHA-256 hashes into a single deterministic JSON document. This bundle is intended for operator handoffs, incident response, and audit workflows where the exact replay inputs and the resulting evidence need to be reproduced verbatim.

The bundle layout is:

- `transactions`: replayed submission evidence with sequence number, wallet, asset pair, score, timestamp, acceptance state, and optional rejection code.
- `events`: replay-emitted event records with sequence number, kind, wallet, asset pair, and a human-readable message.
- `config_snapshot`: a compact view of the replay configuration (admin/service identifiers and replay defaults).
- `issue_references`: normalized issue/incident references supplied via `--issue-ref` flags.
- `hashes`: SHA-256 hashes for the transactions, events, config snapshot, issue references, and the bundle as a whole.

The bundle is deterministic: duplicate or out-of-order issues are normalized before hashing, and the same replay input yields the same bundle payload and hashes across reruns.

## 8. Audit Trail Use Cases

- **Compliance audits**: Prove that an admin upgrade was authorized and correctly ordered
- **Incident response**: Reconstruct the sequence of governance actions leading up to an incident
- **Stakeholder transparency**: Provide auditable evidence of governance decisions
- **Fork arbitration**: If a governance dispute arises, the audit chain provides a cryptographic tie-breaker

## 9. Merkle Second-Preimage & Tree-Shape Ambiguity Audit (Issue #1190)

This section records the review conclusions for the Merkle/hash-chain constructions used by the batch attestation and audit chain, covering leaf/node domain separation, odd-node handling, and commitment of leaf count or depth.

### 9.1 Audit chain (`LedgerLensScoreContract`)

The audit chain is a **linear hash chain**, not a binary Merkle tree:

```
new_root = sha256(old_root || action_hash)
```

- **Domain separation:** The chain has no leaf/internal-node duality — every step is `sha256(prev_root || action_hash)`. There is no second-preimage ambiguity between a "leaf" and an "internal node" because there is only one node type. The genesis root is `sha256([0; 32])`, which is a fixed 32-byte value and cannot collide with a well-formed `action_hash` preimage under SHA-256.
- **Odd-node handling:** Not applicable — the chain is strictly sequential and never pairs nodes.
- **Leaf count / depth commitment:** The chain length is implicitly committed by the final root: replaying a different number of actions yields a different root. Truncation is detectable because the verifier replays the full event list and compares the final root; an attacker cannot present a shorter chain that hashes to the same root without a SHA-256 collision.
- **Conclusion:** No second-preimage or tree-shape ambiguity weakness. No encoding change is required, and historical proofs remain verifiable under the existing schema.

### 9.2 Batch attestation Merkle tree

The batch attestation tree hashes leaves and internal nodes. The review confirmed the following properties are required and must be preserved:

- **Leaf/node domain separation:** Leaves are hashed with a distinct leaf prefix and internal nodes with a distinct node prefix (e.g. `sha256(0x00 || leaf)` vs `sha256(0x01 || left || right)`). Without this, a leaf preimage can be reinterpreted as an internal node preimage (second-preimage attack).
- **Odd-node handling:** When a level has an odd number of nodes, the last node is promoted unchanged to the next level (or duplicated) — the chosen rule must be fixed and documented so that tree shape is unambiguous.
- **Leaf count / depth commitment:** The proof must commit to the leaf count (or tree depth) so that truncated or extended proofs are rejected. A proof that omits trailing leaves must not verify against the same root.

### 9.3 Adversarial test cases

The malformed proof corpus is extended with the following cases, named after the attack:

- `second_preimage_leaf_equals_internal_node`: constructs a fake leaf whose preimage equals an internal node preimage and asserts verification fails.
- `truncated_proof_missing_leaf`: removes a leaf from the proof and asserts verification fails.
- `extended_proof_extra_leaf`: appends an extra leaf to the proof and asserts verification fails.
- `odd_node_promotion_ambiguity`: builds a tree with an odd leaf count and asserts the promoted node cannot be reinterpreted as a different tree shape.

### 9.4 Versioned change and cross-version compatibility

If a domain-separation or leaf-count commitment change is required, it is introduced as a **versioned** encoding: the new scheme is tagged with a version byte and the verifier accepts both the legacy and the new scheme for historical proofs. Cross-version compatibility tests assert that:

- A proof produced under the legacy scheme still verifies under the new verifier.
- A proof produced under the new scheme verifies under the new verifier.
- A legacy proof cannot be reinterpreted as a new-scheme proof (and vice versa) without detection.

For the audit chain, no versioned change is needed (see 9.1); the existing schema is retained and historical proofs remain verifiable.
