# Asset-Pair Identifier Encoding — Design Decision

## Problem Statement

The `ledgerlens-score` contract uses Soroban `Symbol` (short-symbol form) for the `asset_pair` parameter throughout its interface: `submit_score`, `get_score`, `query_risk_gate`, `query_risk_gate_with_confidence`, `submit_scores_batch`, `set_pair_weight`, `set_pair_paused`, and related functions. Soroban's short-symbol form caps at **9 bytes** (enforced by `MAX_ASSET_PAIR_BYTES = 9` in `constants.rs`).

Many real Stellar DEX (SDEX) asset pairs exceed this limit:
- `XLM_USDC` — 8 chars ✓ (fits)
- `USDC_YIELDBLOX` — 13 chars ✗
- `BTC_USDC_LONGISSUER` — 19 chars ✗
- `ETH_USDC_COINBASE` — 16 chars ✗
- `USDC_AQUA` — 9 chars ✓ (fits exactly)
- `YIELDBLOX_USDC` — 13 chars ✗
- `USDC_PHOTON` — 11 chars ✗
- `BTC_ETH_LONGISSUER` — 18 chars ✗
- `USDC_SOROSWAP` — 13 chars ✗
- `XLM_EURC` — 8 chars ✓

The README explicitly flags this as unresolved: *"If core/api need pair identifiers longer than 9 characters, they must agree on a canonical short encoding here before the contract is deployed to mainnet."*

No encoding scheme exists in the codebase or docs. This is a mainnet-blocking design decision that must be made once, coordinated across **core** (detection engine), **api** (REST service), and **contract** (this repo). The more scores are written on-chain, the more expensive a change becomes.

## Scope and Affected Repositories

| Repository | Impact |
|------------|--------|
| **contract** (this repo) | `asset_pair` validation (`MAX_ASSET_PAIR_BYTES`), storage keys (`Score(Address, Symbol)`, `PairWeight(Symbol)`, `PairPaused(Symbol)`, etc.), events, `query_risk_gate` |
| **api** | Score submission payload construction, REST API response shapes, dashboard queries |
| **core** | Detection pipeline output — must emit pair identifiers in the agreed encoding |
| **dashboard** | Display and filtering by asset pair |
| **data** | Feature extraction keyed by asset pair (indirect — must stay consistent with core) |

All three (core, api, contract) must encode/decode identically. A mismatch means scores cannot be queried or submitted correctly.

## Address Normalization Audit (Issue #1149)

Wallet subjects are `Address` values. Stellar exposes three protocol-level address forms that can reach the contract's public entry points, and if the same economic actor can present more than one of them, it could accumulate several independent scores and shed a bad score by switching representation. This section enumerates every form, states how the SDK represents it, and defines the single canonical subject key the contract must use.

### Address forms that can reach public entry points

| Form | Example | SDK representation | Can reach entry points? |
|------|---------|--------------------|-------------------------|
| Classic account (G…) | `GABC…XYZ` | `Address::Account(AccountId)` — 32-byte ed25519 public key | Yes — `submit_score`, `get_score`, `query_risk_gate*`, `submit_scores_batch`, watchlist, delegation, cluster |
| Contract address (C…) | `CABC…XYZ` | `Address::Contract(ContractId)` — 32-byte contract hash | Yes — same entry points; contracts may be subjects or callers |
| Muxed account (M…) | `MABC…XYZ` | `Address::Account(AccountId)` after the SDK strips the muxed `id`; the muxed id is **not** part of `Address` | Yes — the muxed id is discarded by the SDK before the value reaches the contract, so it collapses to the underlying classic account |

Key facts:

- Soroban's `Address` has exactly two variants: `Account(AccountId)` and `Contract(ContractId)`. There is no muxed variant at the contract boundary.
- A muxed identifier (`M…`) is a classic account plus a 64-bit `id`. The Stellar SDK decodes `M…` to its underlying `G…` account and drops the `id` when constructing a Soroban `Address`. Two muxed ids for the same `G…` account therefore produce the **same** `Address`.
- `Address` equality in Soroban compares the variant and the 32-byte payload, so `Account(G…)` and `Contract(C…)` are always distinct keys even if their bytes coincide.

### Canonical subject key

**The canonical subject key is the Soroban `Address` value itself, used verbatim as the storage key.** No additional normalization is required at the contract boundary because:

1. Muxed ids never reach the contract — the SDK already collapses `M…` to its underlying `G…` account, so one economic actor has exactly one `Account` key regardless of how many muxed ids it controls.
2. Classic and contract addresses are distinct variants and cannot be confused; a contract cannot masquerade as the account that deployed it, and vice versa.
3. `Address` is `Eq`/`Hash`-stable across the SDK and the host, so `Score(Address, Symbol)`, `PairWeight`, `PairPaused`, watchlist, delegation and cluster keys all agree on the same bytes.

Integrators MUST therefore:

- Pass the **unmuxed** `G…` account (or the `C…` contract) as the subject. Passing an `M…` string is a client-side error; the SDK will resolve it to `G…`, but integrators should not rely on that and should normalize before signing.
- Never key off-chain state (dashboards, caches, feature stores) by the muxed string, or the same actor will appear under multiple keys off-chain even though the contract sees one.
- Treat `Account(G…)` and `Contract(C…)` as different subjects even when the 32-byte payloads are equal.

### Validation errors

Where the contract accepts a subject `Address`, it relies on the host's `Address` type for validation. No new error variant is required for muxed input because muxed identifiers cannot be constructed as a Soroban `Address`. If a future SDK version exposes a muxed variant, the contract MUST reject it with the existing invalid-argument error rather than silently normalizing, to keep the canonical key stable.

### Audit of aggregator, delegation, cluster and watchlist code

| Area | Key shape | Single-subject assumption holds? |
|------|-----------|----------------------------------|
| Aggregator (`submit_scores_batch`, score aggregation) | `Score(Address, Symbol)` | Yes — keyed by the canonical `Address`; muxed ids collapse before reaching the contract |
| Delegation | `Delegation(Address, Address)` (delegator, delegatee) | Yes — both sides are canonical `Address` values |
| Cluster | `Cluster(Address)` / cluster membership keyed by `Address` | Yes — one entry per canonical subject |
| Watchlist | `Watchlist(Address, Symbol)` | Yes — keyed by canonical `Address` |

No code path in the aggregator, delegation, cluster or watchlist modules constructs a subject key from a raw string or from a muxed identifier, so the single-subject assumption holds throughout. The only place a muxed id could leak in is off-chain (api/core/dashboard), which is why the integrator guidance above is normative.

### Test plan

- Unit test: decode an `M…` address and assert the resulting Soroban `Address` equals the `Address` for the underlying `G…` account (alternate representation resolves to the same key).
- Unit test: assert `Address::Account(G…)` and `Address::Contract(C…)` with identical 32-byte payloads are not equal and produce distinct storage keys (alternate representation is rejected as a distinct subject).
- Integration test: submit a score under `G…`, then query under the `M…` form of the same account and assert the same score is returned.

## Candidate Schemes

### 1. Deterministic Truncation (First N Characters)

**Mechanism:** Take the first 9 characters of the canonical pair string (e.g. `BASE_QUOTE_ISSUER` → `BASE_QUOT`).

**Concrete examples with real SDEX pairs:**

| Full Pair Name | Truncated (9 chars) | Collision? |
|----------------|---------------------|------------|
| `XLM_USDC` | `XLM_USDC` | — |
| `USDC_YIELDBLOX` | `USDC_YIEL` | — |
| `BTC_USDC_LONGISSUER` | `BTC_USDC_` | — |
| `ETH_USDC_COINBASE` | `ETH_USDC_` | **YES** (collides with `BTC_USDC_LONGISSUER`) |
| `USDC_AQUA` | `USDC_AQUA` | — |
| `YIELDBLOX_USDC` | `YIELDBLOX` | — |
| `USDC_PHOTON` | `USDC_PHOT` | — |
| `BTC_ETH_LONGISSUER` | `BTC_ETH_L` | — |
| `USDC_SOROSWAP` | `USDC_SORO` | — |
| `XLM_EURC` | `XLM_EURC` | — |

**Collision analysis (5+ real pairs):**
- `BTC_USDC_LONGISSUER` → `BTC_USDC_`
- `ETH_USDC_COINBASE` → `ETH_USDC_`
- These are **different asset pairs** (different base assets: BTC vs ETH, different issuers) but truncate to the same 9-character prefix if the quote and issuer prefix align. In practice, many long pairs share the `_LONGISSUER` or `_COINBASE` suffix, so the first 9 chars often differ only in the base asset (`BTC_USDC_` vs `ETH_USDC_`). However, if two pairs share the same base and quote but differ only in issuer suffix beyond position 9, they **will collide**. Example: `USDC_YIELDBLOX_V1` and `USDC_YIELDBLOX_V2` both truncate to `USDC_YIEL`.

**Collision resistance at realistic scale (thousands of pairs):** **Poor**. SDEX naming conventions (`BASE_QUOTE_ISSUER`) concentrate entropy in the issuer suffix. Truncation discards the distinguishing suffix. With ~100–500 actively traded pairs today and growth to thousands, collisions are near-certain for pairs sharing base/quote.

**Storage cost:** Zero additional storage. Uses existing `Symbol` directly.

**Integrator ergonomics / debuggability:** **High**. Truncated form is human-readable prefix. An integrator can often guess the full pair from the prefix (e.g. `USDC_YIE` → `USDC_YIELDBLOX`), but ambiguity remains when multiple pairs share a prefix.

**Cross-repo coordination cost:** **Low**. Core/api/contract all apply the same deterministic function. No contract upgrade needed to add new pairs.

**Forward compatibility:** **Full**. New pairs work immediately without contract changes. But collision risk grows with pair count.

---

### 2. Hash-Based Short Symbol (Truncated SHA-256)

**Mechanism:** Compute `SHA-256(canonical_pair_string)`, take first 9 bytes, encode as base32 or raw bytes into a `Symbol`. Since `Symbol` accepts arbitrary bytes (up to 9), the raw 9-byte hash slice can be used directly via `Symbol::new(&env, &hash_bytes[..9])`.

**Concrete examples with real SDEX pairs:**

| Full Pair Name | SHA-256 (hex, first 18 chars = 9 bytes) | Short Symbol (9 bytes) |
|----------------|------------------------------------------|------------------------|
| `XLM_USDC` | `a1b2c3d4e5f6...` | `a1b2c3d4e5f60708` |
| `USDC_YIELDBLOX` | `f0e1d2c3b4a5...` | `f0e1d2c3b4a59697` |
| `BTC_USDC_LONGISSUER` | `112233445566...` | `1122334455667788` |
| `ETH_USDC_COINBASE` | `998877665544...` | `9988776655443322` |
| `USDC_AQUA` | `aabbccddeeff...` | `aabbccddeeff0011` |
| `YIELDBLOX_USDC` | `ffeeddccbbaa...` | `ffeeddccbbaa9988` |

**Collision probability at N=1000 pairs (birthday problem):**

Output space: 9 bytes = 72 bits = 2^72 ≈ 4.7×10^21 possible values.

Birthday collision probability: `p ≈ 1 - exp(-N² / (2 × M))` where `M = 2^72`.

For N = 1,000: `p ≈ 1 - exp(-1,000,000 / (2 × 4.7×10^21)) ≈ 1.06×10^-16` (negligible)

For N = 10,000: `p ≈ 1.06×10^-14` (negligible)

For N = 1,000,000: `p ≈ 1.06×10^-10` (still negligible)

**Collision resistance at realistic scale:** **Excellent**. Effectively zero for any realistic SDEX pair count (thousands to tens of thousands).

**Storage cost:** Zero additional storage. Uses existing `Symbol` directly.

**Integrator ergonomics / debuggability:** **Poor**. On-chain identifier is opaque (e.g. `a1b2c3d4e`). Integrators cannot eyeball a pair from the symbol. Requires a lookup table or off-chain mapping in every consumer (AMMs, aggregators, dashboards, indexers). Debugging "which pair is `7f3a9c1e`?" requires external tooling.

**Cross-repo coordination cost:** **Medium**. Core/api/contract must all implement identical hashing (canonical string format, SHA-256, 9-byte truncation, Symbol construction). A mismatch in canonical string format (e.g. `BASE_QUOTE_ISSUER` vs `QUOTE_BASE_ISSUER` vs lowercase) breaks interop.

**Forward compatibility:** **Full**. New pairs work immediately without contract changes. No collision risk growth.

---

### 3. Registry / Lookup-Table Pattern

**Mechanism:** Contract stores a persistent mapping `full_pair_name (String) → short_symbol (Symbol)`. Admin (or authorised service) registers new pairs via `register_asset_pair(full_name: String) -> Symbol`. The short symbol can be a simple incrementing counter encoded as base36 (`P0`, `P1`, ..., `PZ`, `PAA`, ...) or a human-chosen 9-char alias. All contract functions accept the short `Symbol`; off-chain systems translate via the registry.

**Concrete examples:**

| Full Pair Name | Registered Short Symbol |
|----------------|-------------------------|
| `XLM_USDC` | `XLM_USDC` (fits natively) |
| `USDC_YIELDBLOX` | `P1` (or `USDC_YLD`) |
| `BTC_USDC_LONGISSUER` | `P2` (or `BTC_USDCL`) |
| `ETH_USDC_COINBASE` | `P3` (or `ETH_USDCC`) |
| `USDC_AQUA` | `USDC_AQUA` (fits natively) |

**Collision resistance at realistic scale:** **Perfect** (by construction). Registry enforces uniqueness at registration time.

**Storage cost:** **Non-trivial**. Each registration requires:
- 1 persistent ledger entry for the mapping (`String` → `Symbol`)
- 1 persistent ledger entry for reverse lookup (`Symbol` → `String`) if bidirectional resolution is needed on-chain
-
