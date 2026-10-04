//! Conformance of the reference provider (`contracts/reference-provider`) to the
//! draft On-Chain Risk Score Registry Interface.
//!
//! The reference provider is the specification's executable form: if it fails a
//! `must` vector, the specification and its reference implementation disagree,
//! and the specification is what should change.

mod common;

use common::reference_provider::Adapter;
use risk_registry_conformance::{run, Level, Outcome, RiskProvider};

#[test]
fn reference_provider_conforms() {
    let mut adapter = Adapter::new();
    let report = run(&mut adapter);
    assert!(report.is_conformant(), "{}", report.render());
}

#[test]
fn reference_provider_runs_every_core_vector() {
    let mut adapter = Adapter::new();
    let report = run(&mut adapter);
    let (total, _, _, skipped) = report.counts();
    assert!(total > 0, "the vector file must not be empty");
    assert_eq!(
        skipped,
        0,
        "the reference provider advertises every optional capability, so no vector may be \
         skipped: {}",
        report.render()
    );
}

#[test]
fn reference_provider_advertises_the_root_capability() {
    let mut adapter = Adapter::new();
    adapter.reset();
    assert!(adapter.supports("risk").expect("supports_interface must answer"));
    assert!(adapter.supports("gate").expect("supports_interface must answer"));
    assert!(adapter.supports("cgate").expect("supports_interface must answer"));
    assert!(adapter.supports("score").expect("supports_interface must answer"));
    assert!(adapter.supports("meta").expect("supports_interface must answer"));
    assert!(!adapter.supports("nope").expect("supports_interface must answer"));
    assert!(!adapter.supports("").expect("supports_interface must answer"));
}

#[test]
fn should_level_deviations_do_not_break_conformance() {
    // Guards the harness's own contract: a `should` failure is reported but is
    // not a conformance failure. If this ever breaks, providers will start
    // treating advisory vectors as mandatory.
    let mut adapter = Adapter::new();
    let report = run(&mut adapter);
    for result in &report.results {
        if result.level == Level::Should {
            assert!(
                !matches!(result.outcome, Outcome::Fail(_)),
                "the reference provider should not deviate on `{}`",
                result.id
            );
        }
    }
}
