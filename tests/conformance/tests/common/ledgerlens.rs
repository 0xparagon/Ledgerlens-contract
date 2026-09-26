#![allow(dead_code)]

//! Adapter binding the conformance harness to `ledgerlens-score`.
//!
//! This is the important half of the suite: it demonstrates that the *existing*
//! `ILedgerLensScore` interface — the one already published in
//! `docs/interface-spec.md` and consumed by the mock AMM and mock lending
//! protocol — satisfies the draft standard's core requirements without any ABI
//! change. See `docs/standards/ledgerlens-alignment.md`.

use ::ledgerlens_score::{Error, LedgerLensScoreContract, LedgerLensScoreContractClient};
use risk_registry_conformance::{ProbeError, ProbeResult, RiskProvider, ScoreView, BASE_TIMESTAMP};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Env, Symbol, Vec,
};

use super::map;

/// Model version this adapter publishes. Arbitrary but fixed, so the
/// `model_version` round-trip vector has something to assert.
const MODEL_VERSION: u32 = 1;

/// Drives one `ledgerlens-score` deployment through the conformance vectors.
pub struct Adapter {
    env: Env,
    /// Address the adapter currently believes the provider lives at. Swapped for
    /// a contract-less address to simulate an unreachable provider.
    registry: Address,
    healthy: Address,
    unreachable: Address,
    scope: Symbol,
}

impl Adapter {
    pub fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let healthy = deploy(&env);
        let unreachable = Address::generate(&env);
        Adapter {
            env,
            registry: healthy.clone(),
            healthy,
            unreachable,
            scope: symbol_short!("XLM_USDC"),
        }
    }
}

impl Default for Adapter {
    fn default() -> Self {
        Adapter::new()
    }
}

impl RiskProvider for Adapter {
    fn provider_id(&self) -> &str {
        "ledgerlens-score"
    }

    fn reset(&mut self) -> Address {
        self.env = Env::default();
        self.env.mock_all_auths();
        self.healthy = deploy(&self.env);
        self.registry = self.healthy.clone();
        Address::generate(&self.env)
    }

    fn seed_score(&mut self, score: u32, confidence: u32, age_secs: u64) -> Address {
        let subject = Address::generate(&self.env);
        // Publish at the fixed base timestamp, then move the chain clock forward
        // to create the requested age. Advancing the clock without advancing the
        // ledger sequence is deliberate: score TTLs are measured in ledgers, so
        // this isolates staleness from expiry.
        self.env.ledger().with_mut(|ledger| ledger.timestamp = BASE_TIMESTAMP);
        let client = LedgerLensScoreContractClient::new(&self.env, &self.registry);
        client.submit_score(
            &Vec::new(&self.env),
            &subject,
            &self.scope,
            &score,
            &false,
            &false,
            &BASE_TIMESTAMP,
            &confidence,
            &MODEL_VERSION,
            &None,
        );
        if age_secs > 0 {
            self.env.ledger().with_mut(|ledger| ledger.timestamp = BASE_TIMESTAMP + age_secs);
        }
        subject
    }

    fn now(&self) -> u64 {
        self.env.ledger().timestamp()
    }

    fn make_unavailable(&mut self) {
        self.registry = self.unreachable.clone();
    }

    fn make_available(&mut self) {
        self.registry = self.healthy.clone();
    }

    fn gate(&mut self, subject: &Address, threshold: u32) -> ProbeResult<bool> {
        let client = LedgerLensScoreContractClient::new(&self.env, &self.registry);
        map(client.try_query_risk_gate(subject, &self.scope, &threshold))
    }

    fn gate_with_confidence(
        &mut self,
        subject: &Address,
        threshold: u32,
        min_confidence: u32,
    ) -> ProbeResult<bool> {
        let client = LedgerLensScoreContractClient::new(&self.env, &self.registry);
        map(client.try_query_risk_gate_with_confidence(
            subject,
            &self.scope,
            &threshold,
            &min_confidence,
        ))
    }

    fn score(&mut self, subject: &Address) -> ProbeResult<ScoreView> {
        let client = LedgerLensScoreContractClient::new(&self.env, &self.registry);
        match client.try_get_score(subject, &self.scope) {
            Ok(Ok(score)) => Ok(ScoreView {
                score: score.score,
                confidence: score.confidence,
                timestamp: score.timestamp,
                model_version: score.model_version,
            }),
            Ok(Err(Error::ScoreNotFound)) => Err(ProbeError::NotFound),
            Ok(Err(_)) => Err(ProbeError::Rejected),
            Err(_) => Err(ProbeError::Unavailable),
        }
    }

    fn supports(&mut self, capability: &str) -> ProbeResult<bool> {
        let client = LedgerLensScoreContractClient::new(&self.env, &self.registry);
        let symbol = Symbol::new(&self.env, capability);
        map(client.try_supports_interface(&symbol))
    }

    fn metadata_capabilities(&mut self) -> ProbeResult<std::vec::Vec<String>> {
        let client = LedgerLensScoreContractClient::new(&self.env, &self.registry);
        let metadata = map(client.try_get_interface_metadata())?;
        let capabilities = metadata.capabilities.iter().map(|symbol| symbol.to_string()).collect();
        Ok(capabilities)
    }

    fn interface_version(&mut self) -> ProbeResult<u32> {
        let client = LedgerLensScoreContractClient::new(&self.env, &self.registry);
        let metadata = map(client.try_get_interface_metadata())?;
        Ok(metadata.interface_version)
    }

    fn event_count(&mut self) -> usize {
        self.env.events().all().len()
    }
}

fn deploy(env: &Env) -> Address {
    let contract_id = env.register_contract(None, LedgerLensScoreContract);
    let client = LedgerLensScoreContractClient::new(env, &contract_id);
    client.initialize(&Address::generate(env), &Address::generate(env));
    contract_id
}
