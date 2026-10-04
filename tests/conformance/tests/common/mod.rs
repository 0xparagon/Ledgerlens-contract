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
/// A `try_` client call has *two* nested results and the layers mean opposite
/// things from what they look like:
///
/// ```text
/// Result<Result<T, ConversionError>, Result<E, InvokeError>>
///   Ok(Ok(v))              call returned a value                   -> Ok(v)
///   Ok(Err(_))             returned, but host could not decode it   -> Unavailable
///   Err(Ok(e))             the provider returned Err(e)            -> e.probe_error()
///   Err(Err(Contract(6)))  provider refused; 6 == ScoreNotFound    -> NotFound
///   Err(Err(_))            trapped, or nothing deployed there      -> Unavailable
/// ```
///
/// Collapsing those into one `is_err()` is the mistake this standard exists to
/// prevent: a trapped call and a provider saying "no data" become
/// indistinguishable, and a fail-open consumer reads the first as safe. `map` is
/// the single place that distinction is made, for every provider.
///
/// The `Contract(6)` arm is the reason the standard pins error *numbers*
/// (clause 4.3). A read declared `-> bool` has no error channel of its own, so a
/// provider that needs to refuse must `panic_with_error!`, and the only thing
/// that survives the host boundary is the discriminant.
pub fn map<T, E: ProviderError>(
    outcome: Result<Result<T, soroban_sdk::ConversionError>, Result<E, soroban_sdk::InvokeError>>,
) -> Result<T, risk_registry_conformance::ProbeError> {
    use risk_registry_conformance::ProbeError;
    match outcome {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) => Err(ProbeError::Unavailable),
        Err(Ok(error)) => Err(error.probe_error()),
        Err(Err(soroban_sdk::InvokeError::Contract(code)))
            if code == risk_registry_conformance::NOT_FOUND_CODE =>
        {
            Err(ProbeError::NotFound)
        }
        Err(Err(_)) => Err(ProbeError::Unavailable),
    }
}

/// A provider's own error enum, classified into the harness's three outcomes.
///
/// The only classification that is not mechanical is "no data for this subject",
/// which must become [`NotFound`](risk_registry_conformance::ProbeError::NotFound)
/// rather than `Rejected`: the harness asserts the two are treated differently,
/// and the reason is that only one of them is worth retrying and neither is safe
/// to read as a low score.
pub trait ProviderError {
    fn probe_error(&self) -> risk_registry_conformance::ProbeError;
}

/// Classification for a read declared `-> bool` or `-> T`, where the provider has
/// no error type of its own and `E` is the bare host error.
///
/// Reaching this arm means the provider signalled a refusal through the host
/// channel rather than through a typed `Result` — which the standard does not
/// permit, and which maps to `Rejected` rather than `Unavailable` on purpose: the
/// provider demonstrably answered, so calling it unreachable would be a lie that
/// hides a real protocol bug.
impl ProviderError for soroban_sdk::Error {
    fn probe_error(&self) -> risk_registry_conformance::ProbeError {
        risk_registry_conformance::ProbeError::Rejected
    }
}
