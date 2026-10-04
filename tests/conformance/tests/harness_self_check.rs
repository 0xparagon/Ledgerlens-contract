//! Self-checks for the conformance harness.
//!
//! A conformance suite that cannot fail is worse than no suite: it manufactures
//! confidence. These tests pin the vector file's integrity and prove the
//! harness detects the two failure modes that matter most — a provider that
//! treats an unknown subject as safe, and a provider whose gate says "safe"
//! while its score payload is unavailable.

use risk_registry_conformance::{
    run, run_vector, suite, Level, Outcome, Probe, ProbeError, ProbeResult, RiskProvider,
    ScoreView, BASE_TIMESTAMP,
};
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn vector_file_is_wellformed() {
    let parsed = suite();
    assert!(!parsed.interface.is_empty(), "interface must be named");
    assert!(!parsed.suite_version.is_empty(), "suite_version must be set");
    assert!(!parsed.vectors.is_empty(), "the suite must have vectors");

    let mut ids: Vec<&str> = parsed.vectors.iter().map(|vector| vector.id.as_str()).collect();
    let total = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), total, "vector ids must be unique: {ids:?}");

    let must = parsed.vectors.iter().filter(|vector| vector.level == Level::Must).count();
    assert!(must > 0, "a suite with no must-level vectors cannot gate anything");

    for vector in &parsed.vectors {
        assert!(!vector.clause.is_empty(), "{} must cite a specification clause", vector.id);
        assert!(!vector.title.is_empty(), "{} must have a title", vector.id);
    }
}

#[test]
fn every_capability_gated_vector_names_a_capability() {
    let parsed = suite();
    for vector in &parsed.vectors {
        let needs_optional_read = matches!(vector.probe, Probe::ScorePresent | Probe::ScoreAbsent);
        if needs_optional_read {
            assert_eq!(
                vector.requires_capability.as_deref(),
                Some("score"),
                "{} reads a score, so it must only apply to providers advertising `score`",
                vector.id
            );
        }
    }
}

#[test]
fn harness_detects_a_provider_that_treats_unknown_as_safe() {
    let mut provider = AlwaysSafeProvider::new();
    let report = run(&mut provider);

    assert!(!report.is_conformant(), "the broken provider must not be conformant");

    let failures: Vec<&str> = report
        .results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Fail(_)))
        .map(|result| result.id.as_str())
        .collect();

    assert!(failures.contains(&"CORE-001"), "fail-open on unknown subject must be caught");
    assert!(failures.contains(&"CORE-004"), "non-strict threshold comparison must be caught");
}

#[test]
fn harness_detects_a_gate_that_hides_an_unavailable_payload() {
    let parsed = suite();
    let vector =
        parsed.vectors.iter().find(|vector| vector.id == "CONS-001").expect("CONS-001 must exist");
    let mut provider = AlwaysSafeProvider::new();
    let outcome = run_vector(&mut provider, vector);
    match outcome {
        Outcome::Fail(detail) => {
            assert!(
                detail.contains("deny_provider_unavailable"),
                "the gate claimed safety while the payload was unavailable: {detail}"
            );
        }
        other => panic!("expected a failure for CONS-001, got {other}"),
    }
}

/// A provider that answers "safe" to everything and can never produce a score.
///
/// It exists to prove the suite has teeth. Every read is deliberately wrong in
/// the most dangerous direction: an unknown subject passes the gate, and the
/// score payload is always missing.
struct AlwaysSafeProvider {
    env: Env,
}

impl AlwaysSafeProvider {
    fn new() -> Self {
        AlwaysSafeProvider { env: Env::default() }
    }
}

impl RiskProvider for AlwaysSafeProvider {
    fn provider_id(&self) -> &str {
        "always-safe-anti-example"
    }

    fn reset(&mut self) -> Address {
        Address::generate(&self.env)
    }

    fn seed_score(&mut self, _score: u32, _confidence: u32, _age_secs: u64) -> Address {
        Address::generate(&self.env)
    }

    fn now(&self) -> u64 {
        BASE_TIMESTAMP
    }

    fn make_unavailable(&mut self) {}

    fn make_available(&mut self) {}

    fn gate(&mut self, _subject: &Address, _threshold: u32) -> ProbeResult<bool> {
        Ok(true)
    }

    fn gate_with_confidence(
        &mut self,
        _subject: &Address,
        _threshold: u32,
        _min_confidence: u32,
    ) -> ProbeResult<bool> {
        Ok(true)
    }

    fn score(&mut self, _subject: &Address) -> ProbeResult<ScoreView> {
        Err(ProbeError::NotFound)
    }

    fn supports(&mut self, capability: &str) -> ProbeResult<bool> {
        Ok(matches!(capability, "risk" | "gate" | "cgate" | "score" | "meta"))
    }

    fn metadata_capabilities(&mut self) -> ProbeResult<std::vec::Vec<String>> {
        Ok(vec!["risk".to_string(), "gate".to_string()])
    }

    fn interface_version(&mut self) -> ProbeResult<u32> {
        Ok(1)
    }

    fn event_count(&mut self) -> usize {
        0
    }
}
