//! Shared adapters that bind the conformance harness to concrete providers.
//!
//! Each submodule implements [`risk_registry_conformance::RiskProvider`] for one
//! provider. They are collected here so the harness itself stays
//! provider-neutral, and each is marked `#![allow(dead_code)]` because a test
//! binary only exercises the adapter it needs.

pub mod ledgerlens;
pub mod reference_provider;

/// Translate a Soroban cross-contract call outcome into a probe result.
///
/// The outer `Result` is the host/transport layer: a trap, a missing contract,
/// or an undecodable response. The inner `Result` is the provider's own answer.
/// Collapsing the two is the mistake this standard exists to prevent, so the
/// mapping is done in exactly one place.
pub fn map<T, E>(
    outcome: Result<Result<T, soroban_sdk::ConversionError>, Result<E, soroban_sdk::InvokeError>>,
) -> Result<T, risk_registry_conformance::ProbeError> {
    match outcome {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) => Err(risk_registry_conformance::ProbeError::Rejected),
        Err(_) => Err(risk_registry_conformance::ProbeError::Unavailable),
    }
}
