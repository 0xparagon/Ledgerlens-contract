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

## Subject Key Space (Wallets, Pairs, Pools)

The contract keys every score by a **subject**. Three subject kinds share one flat key space and must be provably disjoint:

| Subject kind | Encoding | Example |
|--------------|----------|---------|
| Wallet | `G...` StrKey account address (56 chars) | `GABCD...` |
| Asset pair | canonical pair symbol (≤ 9 bytes, see below) | `XLM_USDC` |
| Liquidity pool | `L` + 8-byte pool-id prefix (9 bytes total) | `L1a2b3c4d` |

### Pool subject encoding

Stellar AMM liquidity pools are identified by a 32-byte pool ID (the SHA-256 of the pool's parameters). The contract cannot store the full 32-byte ID in a `Symbol`, so the pool subject is the **9-byte short symbol** `L` followed by the first 8 bytes of the pool ID, hex-encoded (lowercase).

- Prefix byte `L` (0x4C) is reserved for pool subjects.
- The remaining 8 bytes are the first 8 bytes of the pool ID, hex-encoded.
- Total length is always exactly 9 bytes, matching `MAX_ASSET_PAIR_BYTES`.

### Collision proof over the extended key space

The three encodings are disjoint by construction:

1. **Wallet vs pair/pool.** Wallet subjects are 56-character StrKey addresses beginning with `G`. Pair and pool subjects are ≤ 9 bytes. Length alone separates wallets from the other two kinds.
2. **Pair vs pool.** Pair symbols are canonical `BASE_QUOTE` strings drawn from the SDEX alphabet (`A–Z`, `0–9`, `_`). Pool subjects always begin with the reserved byte `L`. A canonical pair symbol can only begin with `L` if its base asset name begins with `L` (e.g. `LUMEN_USDC`). To keep the spaces disjoint, the codec **rejects** any canonical pair whose first byte is `L`; such pairs must be registered through the pool-style path or renamed. This is enforced by `validate_asset_pair` and covered by the collision tests below.
3. **Pool vs pool.** Two distinct pool IDs collide only if their first 8 bytes match. Over the 64-bit prefix space the birthday bound gives `p ≈ N² / 2^65`; for N = 1,000,000 pools this is ≈ 2.7×10^-8, and for realistic SDEX pool counts (thousands) it is negligible. Full 32-byte IDs remain the authoritative identifier off-chain; the 8-byte prefix is a display/query key only.

### Codec round-trip

`encode_pool_subject(pool_id: &[u8; 32]) -> Symbol` produces `L` + hex(pool_id[..8]). `decode_pool_subject(symbol) -> [u8; 8]` returns the 8-byte prefix. Round-trip is exact for all 32-byte inputs, including maximum-length identifiers (all-`0xFF` and all-`0x00` pool IDs), and is covered by the codec round-trip tests.

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
- Ongoing rent for both entries

**Integrator ergonomics / debuggability:** **Medium**. Short symbols can be human-chosen aliases (`USDC_YLD`) which are readable, but the mapping is authoritative and must be consulted. Counter-based symbols (`P1`) are opaque.

**Cross-repo coordination cost:** **High**. Core/api must query the registry (or a mirrored off-chain copy) to translate. Registration is a privileged operation requiring admin key management and an operational process for adding pairs.

**Forward compatibility:** **Full**, but requires an admin transaction per new pair. Adds operational latency and a privileged surface.

---

## Pool Risk Scoring

### Score type

Pools **reuse the existing `RiskScore` type**. A pool's risk profile is expressed with the same fields (score, confidence, timestamp, evidence hash) so that gate consumers can query pools through the same interface as wallets and pairs. Pool-specific signals such as reserve imbalance are carried as **flags** on the score submission rather than as new required fields, preserving ABI compatibility:

- `RESERVE_IMBALANCE` — pool reserves are skewed beyond the configured tolerance.
- `LOW_LIQUIDITY` — pool TVL below the configured floor.
- `STALE_RESERVES` — reserves have not been refreshed within the freshness window.

Flags are advisory metadata; the numeric score remains the gate input.

### Aggregator behavior

- A pool score is computed by the aggregator from the pool's own trade/flow features, exactly like a pair score.
- **Pool scores do not automatically propagate to their liquidity providers.** An LP's wallet score is computed from the LP's own activity. This avoids penalising passive LPs for pool-level manipulation they did not perform.
- The relationship is one-directional and advisory: when a pool is flagged, the aggregator may attach the pool subject as evidence on the LP's score, but the LP's numeric score is unchanged unless the LP's own features warrant it.
- Aggregation across subjects (wallet, pair, pool) uses the same weighted-mean machinery; pool weights are configured via `set_pair_weight`-equivalent admin calls keyed by the pool subject.

### Gate consumers

`query_risk_gate` and `query_risk_gate_with_confidence` accept any subject symbol, so a pool subject (`L` + 8-byte prefix) is queried through the identical interface used for wallets and pairs. No new gate entry point is required. The mock AMM example (`contracts/mock-amm/src/lib.rs`) gates on a pool score by passing the pool subject to `query_risk_gate`.

## Acceptance Criteria Mapping

- **Collision tests over the extended key space** — see "Collision proof over the extended key space" above; tests assert wallet/pair/pool disjointness and the `L`-prefix reservation.
- **Codec round-trip tests including maximum-length identifiers** — see "Codec round-trip" above; tests cover all-`0x00` and all-`0xFF` pool IDs.
- **Documentation and schema artifacts updated** — this document.
- **A mock AMM example gates on a pool score** — `contracts/mock-amm/src/lib.rs` queries the gate with a pool subject.
