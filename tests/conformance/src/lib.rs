//! Provider-neutral conformance harness for the **On-Chain Risk Score Registry
//! Interface**.
//!
//! The specification is [`docs/standards/sep-risk-score-registry-interface.md`](../../docs/standards/sep-risk-score-registry-interface.md)
//! (SEP draft). This crate turns its normative requirements into data: the
//! vectors in [`conformance_vectors.json`](../conformance_vectors.json) are
//! provider-neutral, and [`run`] executes them against anything that
//! implements [`RiskProvider`].
//!
//! # Running the suite against your provider
//!
//! 1. Implement [`RiskProvider`] for your provider. The trait is deliberately
//!    small: five reads plus setup helpers.
//! 2. Write a two-line test:
//!
//! ```no_run
//! # use risk_registry_conformance::{run, RiskProvider};
//! # fn check<P: RiskProvider>(mut provider: P) {
//! let report = run(&mut provider);
//! assert!(report.is_conformant(), "{}", report.render());
//! # }
//! ```
//!
//! 3. Treat the JSON file, not this Rust code, as the contract between
//!    implementations. Adding a vector extends every provider's obligations
//!    automatically, and providers written in other languages can translate the
//!    same file without reading any Rust.
//!
//! # Why a harness instead of a shared SDK
//!
//! A provider and its consumer are frequently written by different teams, in
//! different languages, and released on different schedules. Pinning
//! conformance to a shared code library would turn the standard into a library
//! release. A data-only vector file plus a thin adapter per provider keeps the
//! normative surface small and lets a provider claim conformance without
//! adopting anybody's SDK.

use serde::Deserialize;
use soroban_sdk::Address;
use std::fmt;
use std::fmt::Write as _;

/// Ledger timestamp at which every vector's score is published.
///
/// Adapters publish at exactly this timestamp and then advance the chain clock
/// by the vector's `age_secs`, so the expected `timestamp` is a constant and
/// staleness arithmetic is reproducible across providers.
pub const BASE_TIMESTAMP: u64 = 1_700_000_000;

/// The scope (asset pair, market, or jurisdiction) used by every vector.
///
/// Deliberately not a real trading pair: the value only has to be a `Symbol`
/// the provider accepts, so vector files stay portable to providers whose scope
/// vocabulary differs.
pub const SCOPE: &str = "XLM_USDC";

/// Capability symbol for the confidence-aware gate.
pub const CAP_CGATE: &str = "cgate";

/// Capability symbol for the advisory score payload.
pub const CAP_SCORE: &str = "score";

/// Root capability: "this deployment implements this standard".
pub const CAP_ROOT: &str = "risk";

/// The provider-neutral subset of a risk score.
///
/// `score`, `confidence`, and `timestamp` are the three fields a consumer can
/// interpret without provider-specific knowledge. `model_version` is opaque and
/// exists so a consumer can notice that a provider re-based its model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScoreView {
    pub score: u32,
    pub confidence: u32,
    pub timestamp: u64,
    pub model_version: u32,
}

/// Why a probe produced no value.
///
/// The distinction between [`ProbeError::Unavailable`] and
/// [`ProbeError::NotFound`] is the single most important thing this harness
/// checks: collapsing them lets a dead provider masquerade as a safe subject.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeError {
    /// The provider did not answer: the cross-contract call trapped, returned a
    /// host error, the address holds no contract, or the response could not be
    /// decoded. Never interpretable as "low risk".
    Unavailable,
    /// The provider answered, and the answer was "no data for this subject".
    NotFound,
    /// The provider answered with some other domain error (paused, not
    /// initialised, unauthorised caller, out-of-range input).
    Rejected,
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            ProbeError::Unavailable => "unavailable",
            ProbeError::NotFound => "not_found",
            ProbeError::Rejected => "rejected",
        };
        f.write_str(text)
    }
}

/// Result of a single read against a provider.
pub type ProbeResult<T> = Result<T, ProbeError>;

/// What a consumer does when a provider cannot be reached.
///
/// This is the one knob where a consumer can consciously choose availability
/// over risk freshness. It is always explicit: the standard forbids an implicit
/// fail-open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailablePolicy {
    /// Deny. The default, and the only option the standard treats as safe.
    FailClosed,
    /// Allow, because the integrating protocol explicitly chose availability
    /// over risk freshness. Recorded so the choice is auditable, never implicit.
    FailOpen,
}

/// A consumer's risk policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Deny when `score >= threshold`.
    pub threshold: u32,
    /// Deny when `confidence < min_confidence`.
    pub min_confidence: u32,
    /// Deny when `now - timestamp > max_age_secs`. `0` disables the age check,
    /// which is almost never what a value-bearing action wants.
    pub max_age_secs: u64,
    /// What to do when the provider does not answer.
    pub on_unavailable: UnavailablePolicy,
}

/// The reference consumer's decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Proceed with the protected action.
    Allow,
    /// Score is at or above the threshold.
    DenyHighRisk,
    /// Score exists but the model was not confident enough to rely on it.
    DenyLowConfidence,
    /// Score is older than the consumer's freshness bound.
    DenyStale,
    /// No score exists for this subject. Fail-closed, same as high risk.
    DenyNoScore,
    /// The provider did not answer. Kept distinct from a risk rejection so
    /// operators can alert on provider health separately from risk.
    DenyProviderUnavailable,
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Decision::Allow => "allow",
            Decision::DenyHighRisk => "deny_high_risk",
            Decision::DenyLowConfidence => "deny_low_confidence",
            Decision::DenyStale => "deny_stale",
            Decision::DenyNoScore => "deny_no_score",
            Decision::DenyProviderUnavailable => "deny_provider_unavailable",
        };
        f.write_str(text)
    }
}

/// A provider under test.
///
/// Implementations own their `soroban_sdk::Env` and registry address. The
/// harness never assumes how a provider is deployed, only that it answers the
/// five reads below.
pub trait RiskProvider {
    /// Stable identifier used in reports, e.g. `ledgerlens-score`.
    fn provider_id(&self) -> &str;

    /// Deploy a fresh provider with empty state and return a subject address
    /// that has never been scored.
    fn reset(&mut self) -> Address;

    /// Publish `score`/`confidence` for a fresh subject at [`BASE_TIMESTAMP`],
    /// then advance the chain clock by `age_secs`. Returns the subject address.
    fn seed_score(&mut self, score: u32, confidence: u32, age_secs: u64) -> Address;

    /// Current chain time in seconds.
    fn now(&self) -> u64;

    /// Point the adapter at an address with no contract behind it, simulating a
    /// provider that is unreachable.
    fn make_unavailable(&mut self);

    /// Point the adapter back at a healthy provider.
    fn make_available(&mut self);

    /// `query_risk_gate`-equivalent: `Ok(true)` means "a score exists and is
    /// strictly below `threshold`".
    fn gate(&mut self, subject: &Address, threshold: u32) -> ProbeResult<bool>;

    /// `query_risk_gate_with_confidence`-equivalent.
    fn gate_with_confidence(
        &mut self,
        subject: &Address,
        threshold: u32,
        min_confidence: u32,
    ) -> ProbeResult<bool>;

    /// `get_score`-equivalent, mapped into the provider-neutral view.
    fn score(&mut self, subject: &Address) -> ProbeResult<ScoreView>;

    /// `supports_interface`-equivalent.
    fn supports(&mut self, capability: &str) -> ProbeResult<bool>;

    /// Capability symbols listed by `get_interface_metadata`-equivalent, as
    /// strings. Used to check that the two discovery paths agree.
    fn metadata_capabilities(&mut self) -> ProbeResult<std::vec::Vec<String>>;

    /// Interface version reported by the provider, for diagnostics.
    fn interface_version(&mut self) -> ProbeResult<u32>;

    /// Number of contract events emitted so far, used by the read-purity
    /// vector.
    fn event_count(&mut self) -> usize;
}

// ── The reference consumer ───────────────────────────────────────────────────
//
// This is the normative decision procedure from the specification's
// "Consumer obligations" section, written once so every provider is exercised
// against the same consumer logic.

/// Evaluate a consumer policy against a provider.
///
/// Order matters and is part of the specification:
///
/// 1. Ask the gate, preferring the confidence-aware form when the provider
///    advertises `cgate` and degrading to the plain form when it does not.
///    A transport failure is resolved by policy, never by assuming low risk.
/// 2. If the gate denied, classify the reason on a best-effort basis so
///    operators see *why* — a low-confidence denial and a high-risk denial are
///    different incidents.
/// 3. If the gate allowed, apply the freshness bound. A gate that ignores
///    staleness is not wrong; a consumer that ignores staleness is.
pub fn evaluate(provider: &mut dyn RiskProvider, subject: &Address, policy: &Policy) -> Decision {
    let confidence_aware = matches!(provider.supports(CAP_CGATE), Ok(true));
    let gate = if confidence_aware {
        provider.gate_with_confidence(subject, policy.threshold, policy.min_confidence)
    } else {
        provider.gate(subject, policy.threshold)
    };

    let passed = match gate {
        Ok(passed) => passed,
        Err(_) => {
            return match policy.on_unavailable {
                UnavailablePolicy::FailClosed => Decision::DenyProviderUnavailable,
                UnavailablePolicy::FailOpen => Decision::Allow,
            }
        }
    };

    if !passed {
        return classify_denial(provider, subject, policy);
    }

    if policy.max_age_secs == 0 {
        return Decision::Allow;
    }

    match provider.score(subject) {
        Ok(view) => {
            let age = provider.now().saturating_sub(view.timestamp);
            if age > policy.max_age_secs {
                Decision::DenyStale
            } else {
                Decision::Allow
            }
        }
        Err(_) => Decision::DenyProviderUnavailable,
    }
}

/// Attribute a gate denial to a reason. Falls back to `DenyHighRisk` when the
/// reason cannot be established: a denial with an unknown cause is still a
/// denial, so mis-attribution is an observability problem, not a safety one.
///
/// Per the specification's decision procedure, the payload is only consulted when
/// the provider advertises `score` — a provider that does not publish one is not
/// asked to explain itself.
fn classify_denial(
    provider: &mut dyn RiskProvider,
    subject: &Address,
    policy: &Policy,
) -> Decision {
    if !matches!(provider.supports(CAP_SCORE), Ok(true)) {
        return Decision::DenyHighRisk;
    }
    match provider.score(subject) {
        Err(ProbeError::NotFound) => Decision::DenyNoScore,
        Ok(view) => {
            if view.confidence < policy.min_confidence {
                Decision::DenyLowConfidence
            } else {
                Decision::DenyHighRisk
            }
        }
        _ => Decision::DenyHighRisk,
    }
}

// ── Vectors ──────────────────────────────────────────────────────────────────

/// The vector file, embedded so a provider cannot accidentally run a stale copy
/// from a checkout.
pub const VECTORS_JSON: &str = include_str!("../conformance_vectors.json");

/// Normative requirement level of a vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// A provider that fails a `must` vector is not conformant.
    Must,
    /// A provider that fails a `should` vector is still conformant, but the
    /// deviation is reported.
    Should,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Level::Must => "must",
            Level::Should => "should",
        })
    }
}

/// Which read a vector exercises.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Probe {
    /// `query_risk_gate` once.
    Gate,
    /// `query_risk_gate` twice; both answers must match.
    GateDeterministic,
    /// `query_risk_gate` must not emit events.
    GateEmitsNoEvents,
    /// `query_risk_gate_with_confidence`.
    GateWithConfidence,
    /// `get_score` for a subject that has a score.
    ScorePresent,
    /// `get_score` for a subject that has none.
    ScoreAbsent,
    /// `supports_interface` for a named capability.
    Capability,
    /// `supports_interface` for symbols no provider will ever define.
    CapabilityUnknown,
    /// The capability list in `get_interface_metadata` agrees with
    /// `supports_interface`.
    MetadataConsistency,
    /// A non-zero interface version must be reported.
    Version,
    /// The reference consumer's decision for a healthy provider.
    ConsumerDecision,
    /// The reference consumer's decision when the provider is unreachable.
    ConsumerDecisionUnavailable,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ScoredSetup {
    pub score: u32,
    pub confidence: u32,
    #[serde(default)]
    pub age_secs: u64,
}

/// The state a vector requires before it probes.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Precondition {
    /// A subject the provider has never seen.
    #[default]
    UnknownSubject,
    /// A subject with a published score.
    Scored(ScoredSetup),
    /// A provider the consumer cannot reach.
    Unavailable,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Args {
    #[serde(default = "default_threshold")]
    pub threshold: u32,
    #[serde(default)]
    pub min_confidence: u32,
    #[serde(default)]
    pub max_age_secs: u64,
    #[serde(default)]
    pub on_unavailable: Option<String>,
    #[serde(default)]
    pub capability: Option<String>,
}

fn default_threshold() -> u32 {
    75
}

impl Default for Args {
    fn default() -> Self {
        Args {
            threshold: default_threshold(),
            min_confidence: 0,
            max_age_secs: 0,
            on_unavailable: None,
            capability: None,
        }
    }
}

/// What a vector expects the probe to produce.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expect {
    /// The read returns this boolean.
    Bool { value: bool },
    /// The read returns a score whose fields match, published at
    /// [`BASE_TIMESTAMP`].
    Score {
        score: u32,
        confidence: u32,
        #[serde(default)]
        model_version: Option<u32>,
    },
    /// The read returns this `ProbeError` variant (`not_found`,
    /// `unavailable`, or `rejected`).
    Error { error: String },
    /// The read reports a non-zero interface version.
    Version,
    /// The read emits this many events.
    Events { count: usize },
    /// The reference consumer reaches this decision.
    Decision { decision: String },
}

/// One normative conformance case.
#[derive(Clone, Debug, Deserialize)]
pub struct Vector {
    /// Stable identifier, e.g. `CORE-004`. Never reused or renumbered.
    pub id: String,
    pub level: Level,
    /// One-line description shown in reports.
    pub title: String,
    /// Specification clause the vector enforces, e.g. `4.2`.
    pub clause: String,
    /// Capability the provider must advertise for this vector to apply. Absent
    /// means the vector always applies.
    #[serde(default)]
    pub requires_capability: Option<String>,
    pub probe: Probe,
    #[serde(default)]
    pub setup: Precondition,
    #[serde(default)]
    pub args: Args,
    pub expect: Expect,
}

/// The whole vector file.
#[derive(Clone, Debug, Deserialize)]
pub struct Suite {
    pub suite_version: String,
    pub interface: String,
    pub vectors: Vec<Vector>,
}

/// Parse the embedded vector file.
pub fn suite() -> Suite {
    serde_json::from_str(VECTORS_JSON).expect("conformance_vectors.json must parse")
}

// ── Execution ────────────────────────────────────────────────────────────────

/// Outcome of one vector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail(String),
    /// The provider does not advertise the capability this vector needs.
    Skipped,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Outcome::Pass => "pass",
            Outcome::Fail(_) => "fail",
            Outcome::Skipped => "skipped",
        })
    }
}

#[derive(Clone, Debug)]
pub struct VectorResult {
    pub id: String,
    pub level: Level,
    pub clause: String,
    pub title: String,
    pub outcome: Outcome,
}

/// Result of running the suite against one provider.
#[derive(Clone, Debug)]
pub struct Report {
    pub provider: String,
    pub results: Vec<VectorResult>,
}

impl Report {
    /// True when no `must`-level vector failed. `should`-level deviations are
    /// reported but do not break conformance.
    pub fn is_conformant(&self) -> bool {
        !self
            .results
            .iter()
            .any(|result| result.level == Level::Must && matches!(result.outcome, Outcome::Fail(_)))
    }

    /// `(total, passed, failed, skipped)` counts.
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        for result in &self.results {
            match result.outcome {
                Outcome::Pass => passed += 1,
                Outcome::Fail(_) => failed += 1,
                Outcome::Skipped => skipped += 1,
            }
        }
        (self.results.len(), passed, failed, skipped)
    }

    /// Human-readable summary suitable for assertion messages and CI logs.
    pub fn render(&self) -> String {
        let (total, passed, failed, skipped) = self.counts();
        let mut out = format!(
            "provider `{}`: {passed}/{total} passed, {failed} failed, {skipped} skipped\n",
            self.provider
        );
        for result in &self.results {
            if let Outcome::Fail(detail) = &result.outcome {
                let _ =
                    writeln!(out, "  [{}] {} {} — {detail}", result.level, result.id, result.title);
            }
        }
        out
    }
}

/// Run every vector against a provider.
pub fn run(provider: &mut dyn RiskProvider) -> Report {
    let suite = suite();
    let mut results = Vec::new();
    for vector in &suite.vectors {
        let outcome = run_vector(provider, vector);
        results.push(VectorResult {
            id: vector.id.clone(),
            level: vector.level,
            clause: vector.clause.clone(),
            title: vector.title.clone(),
            outcome,
        });
    }
    Report { provider: provider.provider_id().to_string(), results }
}

/// Run a single vector. Public so a provider can bisect a failure.
pub fn run_vector(provider: &mut dyn RiskProvider, vector: &Vector) -> Outcome {
    if let Some(capability) = &vector.requires_capability {
        if !matches!(provider.supports(capability), Ok(true)) {
            return Outcome::Skipped;
        }
    }

    provider.make_available();
    let subject = match &vector.setup {
        Precondition::UnknownSubject => provider.reset(),
        Precondition::Scored(scored) => {
            provider.reset();
            provider.seed_score(scored.score, scored.confidence, scored.age_secs)
        }
        Precondition::Unavailable => {
            let subject = provider.reset();
            provider.make_unavailable();
            subject
        }
    };

    match vector.probe {
        Probe::Gate => expect_bool(provider.gate(&subject, vector.args.threshold), &vector.expect),
        Probe::GateDeterministic => {
            let first = provider.gate(&subject, vector.args.threshold);
            let second = provider.gate(&subject, vector.args.threshold);
            match (&vector.expect, first, second) {
                (Expect::Bool { value }, Ok(a), Ok(b)) if a == b && a == *value => Outcome::Pass,
                (Expect::Bool { value }, Ok(a), Ok(b)) => {
                    Outcome::Fail(format!("expected {value} on both calls, got {a} then {b}"))
                }
                (_, Err(e), _) | (_, _, Err(e)) => Outcome::Fail(format!("gate unavailable: {e}")),
            }
        }
        Probe::GateEmitsNoEvents => {
            let before = provider.event_count();
            let gate = provider.gate(&subject, vector.args.threshold);
            let emitted = provider.event_count().saturating_sub(before);
            match (&vector.expect, &gate) {
                (Expect::Events { count }, Ok(_)) if emitted == *count => Outcome::Pass,
                (_, Err(e)) => Outcome::Fail(format!("gate unavailable: {e}")),
                (_, Ok(_)) => Outcome::Fail(format!(
                    "expected {} events, observed {emitted}",
                    describe(&vector.expect)
                )),
            }
        }
        Probe::GateWithConfidence => expect_bool(
            provider.gate_with_confidence(
                &subject,
                vector.args.threshold,
                vector.args.min_confidence,
            ),
            &vector.expect,
        ),
        Probe::ScorePresent => match provider.score(&subject) {
            Ok(view) => check_score(&view, &vector.expect),
            Err(e) => Outcome::Fail(format!("expected a score, got error {e}")),
        },
        Probe::ScoreAbsent => match (provider.score(&subject), &vector.expect) {
            (Err(ProbeError::NotFound), Expect::Error { error }) if error == "not_found" => {
                Outcome::Pass
            }
            (Err(ProbeError::NotFound), _) => {
                Outcome::Fail(format!("expected {}, got not_found", describe(&vector.expect)))
            }
            (Err(e), _) => Outcome::Fail(format!("expected not_found, got error {e}")),
            (Ok(view), _) => {
                Outcome::Fail(format!("expected not_found, got a score of {}", view.score))
            }
        },
        Probe::Capability => {
            let capability = vector.args.capability.clone().unwrap_or_default();
            expect_bool(provider.supports(&capability), &vector.expect)
        }
        Probe::CapabilityUnknown => {
            // Symbols a conformant provider must reject without trapping. The
            // empty symbol is the classic case: `symbol_short!("")` is
            // constructible, and a naive comparison can panic on it.
            for capability in ["", "nosuchcap", "R1SK", "risk_score", "gate2"] {
                match provider.supports(capability) {
                    Ok(false) => {}
                    Ok(true) => {
                        return Outcome::Fail(format!(
                            "supports_interface reported true for unknown capability `{capability}`"
                        ))
                    }
                    Err(e) => {
                        return Outcome::Fail(format!(
                            "supports_interface failed for `{capability}`: {e}"
                        ))
                    }
                }
            }
            Outcome::Pass
        }
        Probe::MetadataConsistency => {
            // Two discovery paths, one truth. A consumer that reads the metadata
            // list and then calls the function it names must not find a hole —
            // that is the failure mode that turns a metadata read into a runtime
            // trap in someone else's protocol.
            let (consistent, detail) = match provider.metadata_capabilities() {
                Ok(advertised) => {
                    let mut detail = String::new();
                    let mut consistent = true;
                    for capability in &advertised {
                        match provider.supports(capability) {
                            Ok(true) => {}
                            Ok(false) => {
                                consistent = false;
                                detail = format!(
                                    "metadata advertises `{capability}`, but supports_interface \
                                     answers false for it"
                                );
                                break;
                            }
                            Err(e) => {
                                consistent = false;
                                detail =
                                    format!("supports_interface failed for `{capability}`: {e}");
                                break;
                            }
                        }
                    }
                    (consistent, detail)
                }
                Err(e) => (false, format!("get_interface_metadata unavailable: {e}")),
            };
            match (&vector.expect, consistent) {
                (Expect::Bool { value: true }, true) => Outcome::Pass,
                (Expect::Bool { value: true }, false) => Outcome::Fail(if detail.is_empty() {
                    "metadata and supports_interface disagree".to_string()
                } else {
                    detail
                }),
                _ => Outcome::Fail(format!(
                    "expected {}, got metadata consistency {consistent}",
                    describe(&vector.expect)
                )),
            }
        }
        Probe::Version => match (provider.interface_version(), &vector.expect) {
            (Ok(version), Expect::Version) if version > 0 => Outcome::Pass,
            (Ok(version), _) => {
                Outcome::Fail(format!("expected a non-zero interface version, got {version}"))
            }
            (Err(e), _) => Outcome::Fail(format!("interface version unavailable: {e}")),
        },
        Probe::ConsumerDecision | Probe::ConsumerDecisionUnavailable => {
            let decision = evaluate(provider, &subject, &policy_from(vector));
            expect_decision(decision, &vector.expect)
        }
    }
}

fn check_score(view: &ScoreView, expect: &Expect) -> Outcome {
    match expect {
        Expect::Score { score, confidence, model_version } => {
            if view.score != *score || view.confidence != *confidence {
                return Outcome::Fail(format!(
                    "expected score={score} confidence={confidence}, got score={} confidence={}",
                    view.score, view.confidence
                ));
            }
            if view.timestamp != BASE_TIMESTAMP {
                return Outcome::Fail(format!(
                    "expected timestamp {BASE_TIMESTAMP}, got {}",
                    view.timestamp
                ));
            }
            if view.score > 100 || view.confidence > 100 {
                return Outcome::Fail(format!(
                    "score/confidence outside 0..=100: score={} confidence={}",
                    view.score, view.confidence
                ));
            }
            if let Some(expected) = model_version {
                if view.model_version != *expected {
                    return Outcome::Fail(format!(
                        "expected model_version {expected}, got {}",
                        view.model_version
                    ));
                }
            }
            Outcome::Pass
        }
        other => Outcome::Fail(format!("unexpected expectation {other:?}")),
    }
}

fn expect_bool(result: ProbeResult<bool>, expect: &Expect) -> Outcome {
    match (result, expect) {
        (Ok(value), Expect::Bool { value: expected }) if value == *expected => Outcome::Pass,
        (Ok(value), _) => Outcome::Fail(format!("expected {}, got {value}", describe(expect))),
        (Err(e), _) => Outcome::Fail(format!("read failed: {e}")),
    }
}

fn expect_decision(decision: Decision, expect: &Expect) -> Outcome {
    match expect {
        Expect::Decision { decision: expected } if *expected == decision.to_string() => {
            Outcome::Pass
        }
        _ => Outcome::Fail(format!("expected {}, got {decision}", describe(expect))),
    }
}

fn policy_from(vector: &Vector) -> Policy {
    Policy {
        threshold: vector.args.threshold,
        min_confidence: vector.args.min_confidence,
        max_age_secs: vector.args.max_age_secs,
        on_unavailable: match vector.args.on_unavailable.as_deref() {
            Some("fail_open") => UnavailablePolicy::FailOpen,
            _ => UnavailablePolicy::FailClosed,
        },
    }
}

fn describe(expect: &Expect) -> String {
    format!("{expect:?}")
}
