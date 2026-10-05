use std::collections::BTreeMap;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const BPS_DENOMINATOR: u128 = 10_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScoreSubmission {
    pub contract_id: String,
    pub wallet: String,
    pub asset_pair: String,
    pub score: u32,
    pub confidence: u32,
    pub model_version: u32,
    pub timestamp: u64,
    pub nonce: u64,
    pub idempotency_key: String,
}

impl ScoreSubmission {
    pub fn with_derived_idempotency_key(mut self) -> Result<Self> {
        self.idempotency_key = self.derive_idempotency_key()?;
        Ok(self)
    }

    pub fn derive_idempotency_key(&self) -> Result<String> {
        let mut canonical = self.clone();
        canonical.idempotency_key.clear();
        let encoded = serde_json::to_vec(&canonical)?;
        Ok(hex_digest(&encoded))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Resources {
    pub cpu_instructions: u64,
    pub memory_bytes: u64,
    pub read_entries: u32,
    pub write_entries: u32,
}

impl Resources {
    pub fn with_safety_margin(self, margin_bps: u32) -> Self {
        Self {
            cpu_instructions: add_margin(self.cpu_instructions, margin_bps),
            memory_bytes: add_margin(self.memory_bytes, margin_bps),
            read_entries: add_margin(self.read_entries.into(), margin_bps).min(u32::MAX as u64) as u32,
            write_entries: add_margin(self.write_entries.into(), margin_bps).min(u32::MAX as u64) as u32,
        }
    }

    fn contains(self, other: Self) -> bool {
        self.cpu_instructions >= other.cpu_instructions
            && self.memory_bytes >= other.memory_bytes
            && self.read_entries >= other.read_entries
            && self.write_entries >= other.write_entries
    }
}

fn add_margin(value: u64, margin_bps: u32) -> u64 {
    let adjusted = (u128::from(value) * (BPS_DENOMINATOR + u128::from(margin_bps))
        + BPS_DENOMINATOR - 1)
        / BPS_DENOMINATOR;
    adjusted.min(u128::from(u64::MAX)) as u64
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransactionBounds {
    pub min_time: u64,
    pub max_time: u64,
    pub ledger_sequence: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub payload: Vec<u8>,
    pub sequence: u64,
    pub fee: u64,
    pub resources: Resources,
    pub bounds: TransactionBounds,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Simulation {
    pub minimum_fee: u64,
    pub required_resources: Resources,
    pub restore_required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Confirmation {
    pub transaction_hash: String,
    pub ledger: u32,
    pub idempotency_key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    InsufficientFee,
    ResourceLimitExceeded,
    BadSequence,
    ExpiredBounds,
    RestoreRequired,
    RpcLag,
    Permanent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportError {
    pub kind: FailureKind,
    pub message: String,
    pub required_resources: Option<Resources>,
}

impl TransportError {
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), required_resources: None }
    }

    pub fn with_resources(mut self, resources: Resources) -> Self {
        self.required_resources = Some(resources);
        self
    }
}

pub trait SorobanTransport {
    fn next_sequence(&mut self) -> std::result::Result<u64, TransportError>;
    fn transaction_bounds(&mut self) -> std::result::Result<TransactionBounds, TransportError>;
    fn build(
        &mut self,
        request: &ScoreSubmission,
        sequence: u64,
        bounds: TransactionBounds,
        fee: u64,
        resources: Resources,
    ) -> std::result::Result<Transaction, TransportError>;
    fn simulate(&mut self, transaction: &Transaction) -> std::result::Result<Simulation, TransportError>;
    fn sign(&mut self, transaction: &Transaction) -> std::result::Result<Vec<u8>, TransportError>;
    fn submit(&mut self, signed_transaction: &[u8]) -> std::result::Result<String, TransportError>;
    fn confirm(&mut self, transaction_hash: &str) -> std::result::Result<Confirmation, TransportError>;
    fn lookup_idempotency_key(
        &mut self,
        contract_id: &str,
        idempotency_key: &str,
    ) -> std::result::Result<Option<Confirmation>, TransportError>;
    fn restore_archived_entries(
        &mut self,
        request: &ScoreSubmission,
    ) -> std::result::Result<(), TransportError>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitterConfig {
    pub max_retries: u32,
    pub resource_safety_margin_bps: u32,
    pub fee_safety_margin_bps: u32,
    pub base_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

impl Default for SubmitterConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            resource_safety_margin_bps: 1_500,
            fee_safety_margin_bps: 2_000,
            base_backoff_ms: 250,
            max_backoff_ms: 8_000,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SubmitterMetrics {
    pub attempts: u64,
    pub simulations: u64,
    pub submissions: u64,
    pub confirmations: u64,
    pub restores: u64,
    pub retries_by_failure: BTreeMap<String, u64>,
}

pub struct ScoreSubmitter<T> {
    transport: T,
    config: SubmitterConfig,
    metrics: SubmitterMetrics,
}

impl<T: SorobanTransport> ScoreSubmitter<T> {
    pub fn new(transport: T, config: SubmitterConfig) -> Self {
        Self { transport, config, metrics: SubmitterMetrics::default() }
    }

    pub fn metrics(&self) -> &SubmitterMetrics {
        &self.metrics
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    pub fn submit(&mut self, request: &ScoreSubmission) -> Result<Confirmation> {
        if request.idempotency_key.is_empty() {
            return Err(anyhow!("idempotency_key must be supplied by the caller"));
        }
        if let Some(confirmed) = self
            .transport
            .lookup_idempotency_key(&request.contract_id, &request.idempotency_key)
            .map_err(|error| anyhow!("idempotency lookup failed: {}", error.message))?
        {
            self.log("already_confirmed", request, None);
            return Ok(confirmed);
        }

        let mut sequence = self
            .transport
            .next_sequence()
            .map_err(|error| anyhow!("sequence lookup failed: {}", error.message))?;
        let mut fee = 0u64;
        let mut resources = Resources::default();
        let mut last_error = None;

        for attempt in 0..=self.config.max_retries {
            self.metrics.attempts += 1;
            self.log("attempt", request, Some(attempt));
            let bounds = self
                .transport
                .transaction_bounds()
                .map_err(|error| anyhow!("transaction bounds failed: {}", error.message))?;
            let transaction = match self.transport.build(request, sequence, bounds, fee, resources) {
                Ok(transaction) => transaction,
                Err(error) => {
                    if let Some(result) = self.recover_from_error(request, error, attempt, &mut sequence, &mut fee, &mut resources)? {
                        return Ok(result);
                    }
                    continue;
                }
            };

            self.metrics.simulations += 1;
            let simulation = match self.transport.simulate(&transaction) {
                Ok(simulation) => simulation,
                Err(error) => {
                    if let Some(result) = self.recover_from_error(request, error, attempt, &mut sequence, &mut fee, &mut resources)? {
                        return Ok(result);
                    }
                    continue;
                }
            };
            if simulation.restore_required {
                self.restore(request, attempt)?;
                sequence = self.transport.next_sequence().map_err(|error| anyhow!("sequence refresh failed: {}", error.message))?;
                continue;
            }

            let adjusted_resources = simulation
                .required_resources
                .with_safety_margin(self.config.resource_safety_margin_bps);
            let adjusted_fee = add_margin(simulation.minimum_fee, self.config.fee_safety_margin_bps);
            let adjusted = self
                .transport
                .build(request, sequence, bounds, fee.max(adjusted_fee), adjusted_resources)
                .map_err(|error| anyhow!("resource-adjusted transaction build failed: {}", error.message))?;
            self.metrics.simulations += 1;
            let final_simulation = match self.transport.simulate(&adjusted) {
                Ok(simulation) => simulation,
                Err(error) => {
                    if let Some(result) = self.recover_from_error(request, error, attempt, &mut sequence, &mut fee, &mut resources)? {
                        return Ok(result);
                    }
                    continue;
                }
            };
            if final_simulation.restore_required {
                self.restore(request, attempt)?;
                sequence = self.transport.next_sequence().map_err(|error| anyhow!("sequence refresh failed: {}", error.message))?;
                continue;
            }
            if !adjusted_resources.contains(final_simulation.required_resources) {
                resources = final_simulation
                    .required_resources
                    .with_safety_margin(self.config.resource_safety_margin_bps);
                last_error = Some("re-simulation exceeded adjusted resources".to_owned());
                self.retry(FailureKind::ResourceLimitExceeded, attempt)?;
                continue;
            }
            if adjusted.fee < final_simulation.minimum_fee {
                fee = add_margin(final_simulation.minimum_fee, self.config.fee_safety_margin_bps);
                last_error = Some("re-simulation increased minimum fee".to_owned());
                self.retry(FailureKind::InsufficientFee, attempt)?;
                continue;
            }

            let signed = self
                .transport
                .sign(&adjusted)
                .map_err(|error| anyhow!("transaction signing failed: {}", error.message))?;
            self.metrics.submissions += 1;
            let transaction_hash = match self.transport.submit(&signed) {
                Ok(hash) => hash,
                Err(error) => {
                    if let Some(result) = self.recover_from_error(request, error, attempt, &mut sequence, &mut fee, &mut resources)? {
                        return Ok(result);
                    }
                    continue;
                }
            };
            match self.transport.confirm(&transaction_hash) {
                Ok(confirmation) => {
                    self.metrics.confirmations += 1;
                    self.log("confirmed", request, Some(attempt));
                    return Ok(confirmation);
                }
                Err(error) => {
                    if let Some(result) = self.recover_from_error(request, error, attempt, &mut sequence, &mut fee, &mut resources)? {
                        return Ok(result);
                    }
                    last_error = Some("confirmation not yet visible".to_owned());
                }
            }
        }

        Err(anyhow!("bounded retry policy exhausted: {}", last_error.unwrap_or_else(|| "no confirmation".to_owned())))
    }

    fn restore(&mut self, request: &ScoreSubmission, attempt: u32) -> Result<()> {
        self.metrics.restores += 1;
        self.log("restore_required", request, Some(attempt));
        self.transport
            .restore_archived_entries(request)
            .map_err(|error| anyhow!("restore preamble failed: {}", error.message))
    }

    fn recover_from_error(
        &mut self,
        request: &ScoreSubmission,
        error: TransportError,
        attempt: u32,
        sequence: &mut u64,
        fee: &mut u64,
        resources: &mut Resources,
    ) -> Result<Option<Confirmation>> {
        match error.kind {
            FailureKind::InsufficientFee => {
                *fee = fee.saturating_mul(2).max(1);
            }
            FailureKind::ResourceLimitExceeded => {
                let required = error.required_resources.unwrap_or(*resources);
                *resources = required.with_safety_margin(self.config.resource_safety_margin_bps);
            }
            FailureKind::BadSequence => {
                *sequence = self
                    .transport
                    .next_sequence()
                    .map_err(|refresh| anyhow!("bad sequence; refresh failed: {}", refresh.message))?;
            }
            FailureKind::ExpiredBounds => {}
            FailureKind::RestoreRequired => {
                self.restore(request, attempt)?;
                *sequence = self
                    .transport
                    .next_sequence()
                    .map_err(|refresh| anyhow!("restore completed but sequence refresh failed: {}", refresh.message))?;
            }
            FailureKind::RpcLag => {
                if let Some(confirmation) = self
                    .transport
                    .lookup_idempotency_key(&request.contract_id, &request.idempotency_key)
                    .map_err(|lookup| anyhow!("RPC lag idempotency lookup failed: {}", lookup.message))?
                {
                    self.metrics.confirmations += 1;
                    self.log("confirmed_after_rpc_lag", request, Some(attempt));
                    return Ok(Some(confirmation));
                }
            }
            FailureKind::Permanent => {
                return Err(anyhow!("permanent submission failure: {}", error.message));
            }
        }
        *self
            .metrics
            .retries_by_failure
            .entry(format!("{:?}", error.kind).to_ascii_lowercase())
            .or_default() += 1;
        self.log("retry", request, Some(attempt));
        self.retry(error.kind, attempt)?;
        Ok(None)
    }

    fn retry(&self, kind: FailureKind, attempt: u32) -> Result<()> {
        if attempt >= self.config.max_retries {
            return Err(anyhow!("retry limit reached after {kind:?}"));
        }
        let shift = attempt.min(31);
        let delay = self
            .config
            .base_backoff_ms
            .saturating_mul(1u64 << shift)
            .min(self.config.max_backoff_ms);
        thread::sleep(Duration::from_millis(delay));
        Ok(())
    }

    fn log(&self, event: &str, request: &ScoreSubmission, attempt: Option<u32>) {
        eprintln!(
            "{}",
            serde_json::json!({
                "event": event,
                "contract_id": &request.contract_id,
                "idempotency_key": &request.idempotency_key,
                "nonce": request.nonce,
                "attempt": attempt,
                "metrics": &self.metrics,
            })
        );
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn classify_rpc_error(code: &str, message: &str) -> FailureKind {
    let diagnostic = format!("{} {}", code, message).to_ascii_lowercase();
    if (diagnostic.contains("restore") && diagnostic.contains("required"))
        || diagnostic.contains("archived entry")
        || diagnostic.contains("archived footprint")
    {
        FailureKind::RestoreRequired
    } else if diagnostic.contains("insufficient fee")
        || diagnostic.contains("insufficient_fee")
        || diagnostic.contains("fee too low")
    {
        FailureKind::InsufficientFee
    } else if diagnostic.contains("resource")
        && (diagnostic.contains("limit") || diagnostic.contains("exceeded"))
    {
        FailureKind::ResourceLimitExceeded
    } else if diagnostic.contains("bad sequence") || diagnostic.contains("tx_bad_seq") {
        FailureKind::BadSequence
    } else if diagnostic.contains("expired")
        || diagnostic.contains("time bounds")
        || diagnostic.contains("tx_too_late")
    {
        FailureKind::ExpiredBounds
    } else if diagnostic.contains("not found")
        || diagnostic.contains("pending")
        || diagnostic.contains("lag")
        || diagnostic.contains("timeout")
    {
        FailureKind::RpcLag
    } else {
        FailureKind::Permanent
    }
}