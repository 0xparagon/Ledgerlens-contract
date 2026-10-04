//! Conformance of the shipped `ledgerlens-score` contract to the draft On-Chain
//! Risk Score Registry Interface.
//!
//! Passing here is the alignment evidence for
//! `docs/standards/ledgerlens-alignment.md`: the already-published
//! `ILedgerLensScore` surface satisfies the draft standard's core requirements
//! with no ABI change, no storage change, and no new error variant.
//!
//! Known deviations are asserted explicitly rather than left implicit, so that
//! "it still passes" can never quietly absorb a regression.

mod common;

use common::ledgerlens::Adapter;
use risk_registry_conformance::{run, Outcome, RiskProvider};

#[test]
fn ledgerlens_score_conforms() {
    let mut adapter = Adapter::new();
    let report = run(&mut adapter);
    assert!(report.is_conformant(), "{}", report.render());
}

#[test]
fn ledgerlens_score_deviations_are_exactly_the_documented_ones() {
    let mut adapter = Adapter::new();
    let report = run(&mut adapter);
    let deviations: Vec<&str> = report
        .results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Fail(_)))
        .map(|result| result.id.as_str())
        .collect();

    // CORE-017 is the `risk` root capability: a SHOULD in the draft, and the one
    // deliberate gap between the shipped contract and the standard. Adding the
    // symbol is append-only and therefore non-breaking under
    // `docs/interface-versioning-policy.md`; the divergence is recorded in
    // `docs/standards/ledgerlens-alignment.md` with its rationale.
    assert_eq!(deviations, vec!["CORE-017"], "unexpected deviation set: {}", report.render());
}

#[test]
fn ledgerlens_score_advertises_every_core_capability() {
    let mut adapter = Adapter::new();
    adapter.reset();
    for capability in ["gate", "meta", "cgate", "score"] {
        assert!(
            adapter.supports(capability).expect("supports_interface must answer"),
            "ledgerlens-score must advertise `{capability}`"
        );
    }
}

#[test]
fn ledgerlens_score_needs_no_mandatory_capability_added() {
    // The standard deliberately has no `caps` capability, so the shipped
    // contract's pre-existing capability list already covers the mandatory set
    // (`gate`, `meta`). If a future draft adds a mandatory capability, this test
    // is where that shows up.
    let mut adapter = Adapter::new();
    adapter.reset();
    assert!(
        !adapter.supports("caps").expect("supports_interface must answer"),
        "`caps` is not part of the standard; seeing it advertised means the \
         specification and the vectors have drifted"
    );
}
