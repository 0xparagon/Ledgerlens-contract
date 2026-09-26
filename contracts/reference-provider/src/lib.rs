#![no_std]

//! Reference implementation of the **On-Chain Risk Score Registry Interface**.
//!
//! The normative specification lives in
//! [`docs/standards/sep-risk-score-registry-interface.md`](../../docs/standards/sep-risk-score-registry-interface.md)
//! (SEP draft, `Status: Draft`). This contract exists so that the standard is
//! executable rather than aspirational: every MUST in the specification has a
//! line of code here, and `tests/conformance/` runs the shared conformance
//! vectors against this contract as well as against `ledgerlens-score`.
//!
//! # What this contract is (and is not)
//!
//! * It **is** a provider: a registry that publishes one risk score per
//!   `(subject, scope)` and answers reads about it.
//! * It **is not** a detection engine. Scores arrive through `set_score`, which
//!   an authorised producer calls. All model logic is out of scope for the
//!   standard — that is precisely what makes the interface provider-neutral.
//!
//! # Relationship to `ledgerlens-score`
//!
//! The function names, capability symbols, and error discriminants below are
//! deliberately identical to the existing `ILedgerLensScore` surface
//! (`docs/interface-spec.md`) so that a consumer written against one provider
//! works against the other with no code change. The differences are all
//! *subtractions*: this contract has no Benford/ML signal breakdown, no
//! attestation, no multisig, no aggregation, and no hysteresis. That is the
//! point — it is the floor that every provider can be expected to meet.
//!
//! One intentional difference: initialisation is *not* part of the portable
//! surface. `ledgerlens-score` takes `(admin, service)` and this contract takes
//! `(admin)`, because how a provider authorises its producers is a governance
//! choice, not an interface property. A consumer never calls another contract's
//! `initialize`.
//!
//! See `docs/standards/ledgerlens-alignment.md` for the clause-by-clause
//! mapping and the rationale for each divergence.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec,
};

/// Version of the interface specification implemented by this contract.
///
/// Bumped only in lock-step with the specification's own version. Capability
/// discovery — not this constant — is the mechanism consumers should branch
/// on; see `supports_interface`.
pub const INTERFACE_VERSION: u32 = 1;

/// Build version of this reference provider (independent of the interface
/// version, exactly as `CONTRACT_VERSION` is independent of
/// `interface_version` in `ledgerlens-score`).
pub const CONTRACT_VERSION: u32 = 1;

/// Schema version carried in topic 1 of every event this contract emits.
pub const EVENT_VERSION: u32 = 1;

/// Ledger lifetime, in ledgers, applied to a stored score when it is written.
///
/// The specification deliberately says nothing about rent: a provider is free
/// to choose its own retention policy, but a provider that lets scores expire
/// silently MUST expose that through its read surface (this one reports
/// `ScoreNotFound`, which consumers already treat as fail-closed). The gate
/// functions never extend TTL — see `query_risk_gate`.
const SCORE_TTL: u32 = 518_400;

/// The provider-neutral score payload.
///
/// Field order is part of the XDR encoding that consumers decode against and
/// MUST NOT be reordered. Only the four fields a consumer can interpret without
/// provider-specific knowledge are present; a provider MAY append its own
/// fields at the end (see the specification's "Data layout" section).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RiskScore {
    /// Overall risk, `0..=100`. Higher is riskier.
    pub score: u32,
    /// Model confidence, `0..=100`. This is epistemic weight, **not** a
    /// probability of safety: a low-confidence score is not evidence that a
    /// subject is safe.
    pub confidence: u32,
    /// Ledger timestamp of the observation the score was derived from.
    /// Consumers compute staleness as `now - timestamp`; the provider never
    /// decides staleness on the consumer's behalf.
    pub timestamp: u64,
    /// Opaque identifier of the producing model version, for diagnostics and
    /// for detecting that a provider re-based its model under a consumer.
    pub model_version: u32,
}

/// Runtime metadata for capability discovery.
///
/// Structurally identical to `ILedgerLensScore`'s `InterfaceMetadata` so a
/// consumer can decode either provider's metadata with one binding.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceMetadata {
    /// Version of the interface specification this deployment implements.
    pub interface_version: u32,
    /// Build version of this provider.
    pub contract_version: u32,
    /// Capability symbols this deployment answers `true` for, including the
    /// root `risk` capability.
    pub capabilities: Vec<Symbol>,
    /// Semantic guarantees this deployment commits to. `fail_closed` means
    /// every indeterminate condition resolves to "not safe to proceed".
    pub semantic_constraints: Vec<Symbol>,
}

/// Error codes.
///
/// The numeric values are the shared core code space from the specification
/// and are identical to `ILedgerLensScore`'s published discriminants, so a
/// consumer can map "no score" to the same branch regardless of provider.
/// Codes are append-only.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    /// The provider has already been initialised.
    AlreadyInitialized = 1,
    /// Provider has not been initialised.
    NotInitialized = 2,
    /// Caller is not the configured admin/producer.
    Unauthorized = 3,
    /// `score` outside `0..=100`.
    InvalidScore = 4,
    /// `confidence` outside `0..=100`.
    InvalidConfidence = 5,
    /// No score exists for this `(subject, scope)`. **Not** a transport
    /// failure — it is the "no data" answer, and consumers treat it as
    /// fail-closed.
    ScoreNotFound = 6,
    /// Provider-wide circuit breaker is active. Reads fail closed.
    ContractPaused = 7,
    /// `timestamp` is zero, which is never a valid observation time.
    InvalidTimestamp = 25,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum DataKey {
    Admin,
    Paused,
    /// Provider-wide confidence floor, mirroring `set_global_min_confidence`
    /// in `ILedgerLensScore`. The stricter of the caller's floor and this one
    /// always wins.
    GlobalMinConfidence,
    /// Latest score for a `(subject, scope)` pair.
    Score(Address, Symbol),
}

#[contract]
pub struct ReferenceRiskRegistry;

#[contractimpl]
impl ReferenceRiskRegistry {
    /// One-time wiring. Records the admin/producer address.
    ///
    /// A re-initialisation attempt returns [`Error::AlreadyInitialized`] rather
    /// than trapping, matching `ILedgerLensScore`: a rejected privileged call is
    /// a typed answer, and a caller that cannot distinguish "refused" from
    /// "crashed" has learned nothing.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if has_admin(&env) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        Ok(())
    }

    /// Publishes the latest score for `(subject, scope)`.
    ///
    /// Authorised by the admin recorded at `initialize`; the admin's
    /// `require_auth` is the authorisation boundary, so an off-chain pipeline
    /// key is all a producer needs.
    ///
    /// Out-of-range values are rejected rather than clamped: a consumer that
    /// sees a clamped `100` cannot tell "maximum risk" from "garbage input".
    pub fn set_score(
        env: Env,
        subject: Address,
        scope: Symbol,
        score: u32,
        confidence: u32,
        timestamp: u64,
        model_version: u32,
    ) -> Result<(), Error> {
        Self::require_admin_auth(&env)?;
        if score > 100 {
            return Err(Error::InvalidScore);
        }
        if confidence > 100 {
            return Err(Error::InvalidConfidence);
        }
        if timestamp == 0 {
            return Err(Error::InvalidTimestamp);
        }
        let key = DataKey::Score(subject.clone(), scope.clone());
        env.storage()
            .persistent()
            .set(&key, &RiskScore { score, confidence, timestamp, model_version });
        env.storage().persistent().extend_ttl(&key, SCORE_TTL, SCORE_TTL);
        env.events().publish(
            (symbol_short!("score"), EVENT_VERSION, subject, scope),
            (score, confidence, timestamp, model_version),
        );
        Ok(())
    }

    /// Returns the latest score for `(subject, scope)`.
    ///
    /// Fails closed on everything indeterminate: a paused provider, an
    /// uninitialised provider, or a missing entry all return an error rather
    /// than a zero-valued struct. Consumers MUST distinguish the returned
    /// error from a transport failure (see the specification's "Failure
    /// modes") — a transport failure means *the provider did not answer*, a
    /// domain error means *the provider answered "no"*.
    pub fn get_score(env: Env, subject: Address, scope: Symbol) -> Result<RiskScore, Error> {
        Self::require_readable(&env)?;
        env.storage().persistent().get(&DataKey::Score(subject, scope)).ok_or(Error::ScoreNotFound)
    }

    /// Optional convenience form of [`get_score`]: the same payload, with "no
    /// data" expressed as `None` instead of [`Error::ScoreNotFound`].
    ///
    /// Included for surface parity with `ILedgerLensScore`. A consumer that
    /// distinguishes transport failure from a domain error should prefer
    /// `get_score`, because `get_score_opt` cannot report *why* it has no
    /// score: uninitialised and paused resolve to `None` here, whereas
    /// `get_score` reports `NotInitialized` and `ContractPaused`.
    pub fn get_score_opt(env: Env, subject: Address, scope: Symbol) -> Option<RiskScore> {
        Self::lookup(&env, &subject, &scope)
    }

    /// The minimal integration primitive.
    ///
    /// Returns `true` only when a score exists for `(subject, scope)` and that
    /// score is **strictly below** `gate_threshold`.
    ///
    /// Guarantees required by the specification and relied upon by consumers:
    ///
    /// * **Infallible** — returns `bool`, never `Result`. There is no fallible
    ///   variant to handle.
    /// * **Total** — no input, including `gate_threshold > 100` or
    ///   `u32::MAX`, causes a trap. Out-of-range thresholds resolve to `false`
    ///   because no conformant score can be below them.
    /// * **Side-effect free** — no durable state is written and no TTL is
    ///   extended, so an attacker cannot use the gate to grief a consumer's
    ///   rent budget or to pin state open.
    /// * **Fail-closed** — an unknown subject, a paused provider, or an
    ///   uninitialised provider all return `false`.
    ///
    /// This function does not consider confidence. Use
    /// `query_risk_gate_with_confidence` for high-value decisions: a
    /// `score = 30, confidence = 5` result carries almost no information and
    /// must not be read as evidence of safety.
    pub fn query_risk_gate(env: Env, subject: Address, scope: Symbol, gate_threshold: u32) -> bool {
        if gate_threshold > 100 {
            return false;
        }
        match Self::lookup(&env, &subject, &scope) {
            Some(risk) => risk.score < gate_threshold,
            None => false,
        }
    }

    /// Confidence-aware gate: `true` only when a score exists, the score is
    /// strictly below `gate_threshold`, **and** the score's confidence is at
    /// least `max(min_confidence, global_min_confidence)`.
    ///
    /// A score below the effective floor is treated exactly like a missing
    /// score — epistemically, "I barely know" and "I know nothing" warrant the
    /// same consumer behaviour. `min_confidence > 100` resolves to `false` for
    /// every subject, since confidence is bounded to `0..=100`.
    pub fn query_risk_gate_with_confidence(
        env: Env,
        subject: Address,
        scope: Symbol,
        gate_threshold: u32,
        min_confidence: u32,
    ) -> bool {
        if gate_threshold > 100 || min_confidence > 100 {
            return false;
        }
        let effective_floor = core::cmp::max(min_confidence, global_min_confidence(&env));
        match Self::lookup(&env, &subject, &scope) {
            Some(risk) => risk.score < gate_threshold && risk.confidence >= effective_floor,
            None => false,
        }
    }

    /// Capability discovery. Total: any unrecognised symbol, including the
    /// empty symbol, returns `false` rather than trapping.
    ///
    /// | Capability | Level   | Meaning                                        |
    /// |------------|---------|------------------------------------------------|
    /// | `gate`     | MUST    | `query_risk_gate` is available.                 |
    /// | `meta`     | MUST    | `get_interface_metadata` is available.          |
    /// | `cgate`    | SHOULD  | `query_risk_gate_with_confidence` is available. |
    /// | `score`    | SHOULD  | `get_score` is available.                       |
    /// | `risk`     | SHOULD  | This deployment implements this standard.      |
    ///
    /// Discovery is a *function* requirement, not a symbol: there is
    /// deliberately no `caps` capability advertising the availability of
    /// `supports_interface` itself. A caller learns that by calling it, and a
    /// provider that had to declare it would gain nothing but a way to fail the
    /// requirement.
    ///
    /// The capability set is append-only within a major interface version: a
    /// capability, once published, is never removed or repurposed.
    pub fn supports_interface(env: Env, capability: Symbol) -> bool {
        let _ = env;
        capability == symbol_short!("risk")
            || capability == symbol_short!("gate")
            || capability == symbol_short!("cgate")
            || capability == symbol_short!("score")
            || capability == symbol_short!("meta")
    }

    /// Versioned metadata for consumers that would rather fetch the whole
    /// capability set once than probe symbol by symbol.
    pub fn get_interface_metadata(env: Env) -> InterfaceMetadata {
        let mut capabilities = Vec::new(&env);
        capabilities.push_back(symbol_short!("risk"));
        capabilities.push_back(symbol_short!("gate"));
        capabilities.push_back(symbol_short!("cgate"));
        capabilities.push_back(symbol_short!("score"));
        capabilities.push_back(symbol_short!("meta"));

        let mut constraints = Vec::new(&env);
        // `Symbol::new`, not `symbol_short!`: the string is 11 characters and a
        // short symbol caps at 9. Same spelling the shipped contract uses, which
        // is the point — a constraint vocabulary that differs between providers
        // is not a vocabulary.
        constraints.push_back(Symbol::new(&env, "fail_closed"));

        InterfaceMetadata {
            interface_version: INTERFACE_VERSION,
            contract_version: CONTRACT_VERSION,
            capabilities,
            semantic_constraints: constraints,
        }
    }

    /// Build version. Diagnostics and telemetry only — consumers should branch
    /// on `supports_interface`, never on a version comparison.
    ///
    /// Exposed under both names, as `ILedgerLensScore` does, because renaming a
    /// published function is a breaking change and consumers written against
    /// either spelling should keep working.
    pub fn get_version(env: Env) -> u32 {
        let _ = env;
        CONTRACT_VERSION
    }

    /// Alias of [`get_version`], matching `ILedgerLensScore`.
    pub fn get_contract_version(env: Env) -> u32 {
        let _ = env;
        CONTRACT_VERSION
    }

    /// Sets the provider-wide confidence floor.
    pub fn set_global_min_confidence(env: Env, min_confidence: u32) -> Result<(), Error> {
        Self::require_admin_auth(&env)?;
        if min_confidence > 100 {
            return Err(Error::InvalidConfidence);
        }
        env.storage().instance().set(&DataKey::GlobalMinConfidence, &min_confidence);
        Ok(())
    }

    /// Reads the provider-wide confidence floor. `0` when never configured.
    pub fn get_global_min_confidence(env: Env) -> u32 {
        global_min_confidence(&env)
    }

    /// Provider-wide circuit breaker. While paused every read fails closed and
    /// `set_score` is refused: a provider that has lost its data feed must not
    /// keep serving the last score it happened to receive as if it were current.
    pub fn set_paused(env: Env, paused: bool) -> Result<(), Error> {
        Self::require_admin_auth(&env)?;
        env.storage().instance().set(&DataKey::Paused, &paused);
        Ok(())
    }

    /// Whether the circuit breaker is active.
    pub fn is_paused(env: Env) -> bool {
        storage_paused(&env)
    }

    // ── internals ────────────────────────────────────────────────────────────

    /// Shared read path for both gate functions. `None` means "no usable
    /// signal", which every caller must translate into a fail-closed `false`.
    ///
    /// Deliberately uses a non-extending read: a gate call must not mutate the
    /// provider's rent obligations on behalf of whoever calls it.
    fn lookup(env: &Env, subject: &Address, scope: &Symbol) -> Option<RiskScore> {
        if !has_admin(env) || storage_paused(env) {
            return None;
        }
        env.storage().persistent().get(&DataKey::Score(subject.clone(), scope.clone()))
    }

    /// `get_score` and the gate functions must agree on what "no data" means,
    /// so both go through the same initialisation and pause checks; they differ
    /// only in whether the reason is reported or flattened to `None`.
    fn require_readable(env: &Env) -> Result<(), Error> {
        Self::require_initialized(env)?;
        if storage_paused(env) {
            return Err(Error::ContractPaused);
        }
        Ok(())
    }

    fn require_admin_auth(env: &Env) -> Result<(), Error> {
        let admin: Address =
            env.storage().instance().get(&DataKey::Admin).ok_or(Error::NotInitialized)?;
        admin.require_auth();
        Ok(())
    }

    fn require_initialized(env: &Env) -> Result<(), Error> {
        if has_admin(env) {
            Ok(())
        } else {
            Err(Error::NotInitialized)
        }
    }
}

fn has_admin(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Admin)
}

fn storage_paused(env: &Env) -> bool {
    env.storage().instance().get(&DataKey::Paused).unwrap_or(false)
}

fn global_min_confidence(env: &Env) -> u32 {
    env.storage().instance().get::<_, u32>(&DataKey::GlobalMinConfidence).unwrap_or(0)
}
