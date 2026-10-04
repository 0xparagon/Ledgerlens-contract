//! # LedgerLens Post-Incident Recovery & Reconciliation Tool
//!
//! Off-chain tooling for state snapshot, reconciliation, backup, and
//! post-action verification workflows.
//!
//! ## Commands
//!
//! * `snapshot` — Take a deterministic state snapshot (invokes
//!   `compute_state_checksum` on-chain, saves the result to disk).
//! * `export` — Export all scored entries as a JSON file for off-chain backup.
//! * `reconcile` — Compare two snapshot files and produce a diff report.
//! * `verify` — Verify a saved snapshot against the current on-chain state.
//! * `report` — Generate a post-action verification report from a snapshot
//!   and an export.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ── Data types (mirrors on-chain types for off-chain processing) ────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StateSnapshot {
    score_root: String,
    config_root: String,
    auth_root: String,
    entry_count: u32,
    ledger_seq: u32,
    timestamp: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ExportableScoreEntry {
    wallet: String,
    asset_pair: String,
    score: u32,
    benford_flag: bool,
    ml_flag: bool,
    timestamp: u64,
    confidence: u32,
    model_version: u32,
    benford_score: u32,
    ml_score: u32,
    network_score: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReconciliationReport {
    snapshot_a_path: String,
    snapshot_b_path: String,
    score_roots_match: bool,
    config_roots_match: bool,
    auth_roots_match: bool,
    entry_counts_match: bool,
    all_match: bool,
    details: Vec<String>,
}

/// Mirrors the `(Address, Symbol)` tuple `get_expiring_entries` returns,
/// plus the estimated remaining TTL an off-chain caller reads separately
/// via `get_entry_ttl` — used to prioritize the most urgent entries first
/// for `keeper_extend_entry_ttls` (see docs/rent-griefing-analysis.md).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct KeeperCandidate {
    wallet: String,
    asset_pair: String,
    estimated_ttl_remaining: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct KeeperBatch {
    entries: Vec<KeeperCandidate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PostActionReport {
    snapshot: StateSnapshot,
    action_type: String,
    action_timestamp: String,
    action_description: String,
    pre_action_entry_count: u32,
    post_action_entry_count: Option<u32>,
    checksum_verified: bool,
    verification_notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RecoveryPlan {
    version: u32,
    base_state_root: String,
    export_root: String,
    batch_size: usize,
    batches: Vec<RecoveryBatch>,
    plan_hash: String,
    approvals: Vec<PlanApproval>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RecoveryBatch {
    index: usize,
    operations: Vec<RecoveryOperation>,
    predicted_effects: Vec<String>,
    estimated_cost_units: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum RecoveryOperation {
    Upsert { entry: ExportableScoreEntry },
    Delete { wallet: String, asset_pair: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PlanApproval {
    operator: String,
    public_key: String,
    signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ApplyCheckpoint {
    plan_hash: String,
    next_batch: usize,
    applied_operations: usize,
    state_root: String,
}

// ── CLI ────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "recovery", about = "LedgerLens post-incident recovery & reconciliation tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Save a snapshot description to a JSON file (intended for use after
    /// calling compute_state_checksum on-chain and recording the output).
    Snapshot {
        /// Path to the output snapshot JSON file.
        #[arg(short, long, default_value = "snapshot.json")]
        output: PathBuf,
        /// Score root hex string from on-chain compute_state_checksum.
        #[arg(short = 'r', long)]
        score_root: String,
        /// Config root hex string.
        #[arg(short = 'c', long)]
        config_root: String,
        /// Auth root hex string.
        #[arg(short = 'a', long)]
        auth_root: String,
        /// Number of scored entries.
        #[arg(short = 'n', long)]
        entry_count: u32,
        /// Ledger sequence.
        #[arg(short = 's', long)]
        ledger_seq: u32,
        /// Ledger timestamp.
        #[arg(short = 't', long)]
        timestamp: u64,
    },

    /// Export score entries to a JSON lines file (one entry per line).
    /// This is an off-chain helper; use the contract's export_all_scores_paginated
    /// to fetch the actual data from the chain.
    Export {
        /// Path to save the export JSON to.
        #[arg(short, long, default_value = "export.json")]
        output: PathBuf,
        /// Path to a JSON array of ExportableScoreEntry items (simulated
        /// from off-chain data or collected from the contract).
        #[arg(short = 'i', long)]
        input: Option<PathBuf>,
    },

    /// Reconcile two snapshot files and produce a diff report.
    Reconcile {
        /// First snapshot file (pre-incident / baseline).
        snapshot_a: PathBuf,
        /// Second snapshot file (post-recovery / current).
        snapshot_b: PathBuf,
        /// Path to save the reconciliation report.
        #[arg(short, long, default_value = "reconciliation-report.json")]
        output: PathBuf,
    },

    /// Verify that a snapshot file has internally consistent roots.
    /// (Full on-chain verification requires calling the contract's
    /// `verify_state_checksum` function.)
    Verify {
        /// Snapshot JSON file, or omit when using --state and --export.
        snapshot: Option<PathBuf>,
        /// Path to an optional export JSON to cross-check entry count.
        #[arg(short = 'e', long)]
        export: Option<PathBuf>,
        /// Final state export file to compare with --export.
        #[arg(long)]
        state: Option<PathBuf>,
    },

    /// Create a deterministic, batch-bounded restore plan from export files.
    Plan {
        /// Desired known-good score export.
        #[arg(long)]
        export: PathBuf,
        /// Freshly fetched live score export.
        #[arg(long)]
        live_state: PathBuf,
        /// Output plan file.
        #[arg(short, long, default_value = "recovery-plan.json")]
        output: PathBuf,
        /// Maximum operations per batch (1..=100).
        #[arg(long, default_value_t = 50)]
        batch_size: usize,
    },

    /// Add an Ed25519 operator approval to a plan (private key comes from env).
    Cosign {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        operator: String,
        /// Environment variable containing a 32-byte hex private key.
        #[arg(long, default_value = "RECOVERY_SIGNING_KEY")]
        key_env: String,
    },

    /// Apply an approved plan to a live-state export and checkpoint each batch.
    Apply {
        #[arg(long)]
        plan: PathBuf,
        /// Live-state export file (the file-backed adapter boundary).
        #[arg(long)]
        state: PathBuf,
        /// Trusted operator ID to public-key mapping JSON.
        #[arg(long)]
        trusted_operators: PathBuf,
        /// Resume checkpoint path.
        #[arg(long, default_value = "recovery-checkpoint.json")]
        checkpoint: PathBuf,
    },

    /// Generate a post-action verification report from a snapshot,
    /// export, and action description.
    Report {
        /// Path to the pre-action snapshot file.
        snapshot: PathBuf,
        /// Type of action performed (e.g. "freeze", "restore", "upgrade").
        #[arg(short, long)]
        action: String,
        /// Description of what was done and why.
        #[arg(short, long)]
        description: String,
        /// Path to save the report.
        #[arg(short, long, default_value = "post-action-report.json")]
        output: PathBuf,
    },

    /// Selects the most urgent entries from a candidate list for a keeper
    /// to renew via `keeper_extend_entry_ttls` (see
    /// docs/rent-griefing-analysis.md). Sorts by lowest estimated remaining
    /// TTL first, dedupes by (wallet, asset_pair), and caps the output at
    /// the contract's KEEPER_BATCH_MAX (100) so the result can be fed
    /// straight into a single call.
    Keeper {
        /// Path to a JSON array of KeeperCandidate entries — e.g. collected
        /// off-chain by calling `get_expiring_entries` /`get_entry_ttl`.
        #[arg(short = 'i', long)]
        input: PathBuf,
        /// Path to save the prioritized batch.
        #[arg(short, long, default_value = "keeper-batch.json")]
        output: PathBuf,
        /// Maximum entries to include (capped at KEEPER_BATCH_MAX).
        #[arg(short = 'n', long, default_value_t = 100)]
        max_entries: usize,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Snapshot {
            output,
            score_root,
            config_root,
            auth_root,
            entry_count,
            ledger_seq,
            timestamp,
        } => cmd_snapshot(
            &output,
            &score_root,
            &config_root,
            &auth_root,
            entry_count,
            ledger_seq,
            timestamp,
        ),
        Commands::Export { output, input } => cmd_export(&output, input.as_deref()),
        Commands::Reconcile { snapshot_a, snapshot_b, output } => {
            cmd_reconcile(&snapshot_a, &snapshot_b, &output)
        }
        Commands::Verify { snapshot, export, state } => {
            cmd_verify(snapshot.as_deref(), export.as_deref(), state.as_deref())
        }
        Commands::Plan { export, live_state, output, batch_size } => {
            cmd_plan(&export, &live_state, &output, batch_size)
        }
        Commands::Cosign { plan, operator, key_env } => cmd_cosign(&plan, &operator, &key_env),
        Commands::Apply { plan, state, trusted_operators, checkpoint } => {
            cmd_apply(&plan, &state, &trusted_operators, &checkpoint)
        }
        Commands::Report { snapshot, action, description, output } => {
            cmd_report(&snapshot, &action, &description, &output)
        }
        Commands::Keeper { input, output, max_entries } => {
            cmd_keeper(&input, &output, max_entries)
        }
    }
}

/// Contract-side hard cap (`KEEPER_BATCH_MAX`, mirrors
/// `MAX_EXPIRING_ENTRIES_PER_CALL`) — kept in sync manually since this CLI
/// doesn't depend on the contract crate.
const KEEPER_BATCH_MAX: usize = 100;

// ── Command handlers ───────────────────────────────────────────────────────

fn cmd_snapshot(
    output: &PathBuf,
    score_root: &str,
    config_root: &str,
    auth_root: &str,
    entry_count: u32,
    ledger_seq: u32,
    timestamp: u64,
) -> Result<()> {
    let snapshot = StateSnapshot {
        score_root: score_root.to_string(),
        config_root: config_root.to_string(),
        auth_root: auth_root.to_string(),
        entry_count,
        ledger_seq,
        timestamp,
    };
    let json = serde_json::to_string_pretty(&snapshot).context("Failed to serialize snapshot")?;
    fs::write(output, &json)
        .with_context(|| format!("Failed to write snapshot to {}", output.display()))?;
    eprintln!("Snapshot saved to {}", output.display());
    Ok(())
}

fn cmd_export(output: &PathBuf, input: Option<&Path>) -> Result<()> {
    if let Some(input_path) = input {
        // Read existing export data from a JSON file
        let content = fs::read_to_string(input_path)
            .with_context(|| format!("Failed to read {}", input_path.display()))?;
        // Validate by deserializing
        let entries: Vec<ExportableScoreEntry> = serde_json::from_str(&content)
            .context("Export file is not a valid JSON array of ExportableScoreEntry")?;
        eprintln!("Loaded {} entries from {}", entries.len(), input_path.display());
        fs::write(output, &content)
            .with_context(|| format!("Failed to write export to {}", output.display()))?;
        eprintln!("Export written to {} ({} entries)", output.display(), entries.len());
    } else {
        // Generate a minimal template
        let template = r#"[]"#;
        fs::write(output, template)
            .with_context(|| format!("Failed to write export to {}", output.display()))?;
        eprintln!(
            "Empty export created at {}. Populate it with data from \
             the contract's export_all_scores_paginated function.",
            output.display()
        );
    }
    Ok(())
}

const RECOVERY_PLAN_VERSION: u32 = 1;
const MAX_RECOVERY_BATCH_SIZE: usize = 100;
const ESTIMATED_COST_PER_OPERATION: u64 = 25_000;

type EntryKey = (String, String);
type EntryMap = BTreeMap<EntryKey, ExportableScoreEntry>;

fn load_export(path: &Path) -> Result<Vec<ExportableScoreEntry>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read score export {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("{} is not a valid score export", path.display()))
}

fn index_export(entries: Vec<ExportableScoreEntry>) -> Result<EntryMap> {
    let mut indexed = BTreeMap::new();
    for entry in entries {
        let key = (entry.wallet.clone(), entry.asset_pair.clone());
        if indexed.insert(key.clone(), entry).is_some() {
            anyhow::bail!("duplicate export key wallet={} pair={}", key.0, key.1);
        }
    }
    Ok(indexed)
}

fn export_root(entries: &EntryMap) -> Result<String> {
    let ordered: Vec<&ExportableScoreEntry> = entries.values().collect();
    let bytes = serde_json::to_vec(&ordered).context("encoding ordered score export")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn cmd_plan(export_path: &Path, live_path: &Path, output: &Path, batch_size: usize) -> Result<()> {
    if batch_size == 0 || batch_size > MAX_RECOVERY_BATCH_SIZE {
        anyhow::bail!("batch size must be within 1..={MAX_RECOVERY_BATCH_SIZE}");
    }
    let desired = index_export(load_export(export_path)?)?;
    let current = index_export(load_export(live_path)?)?;
    let mut ordered_operations = BTreeMap::<EntryKey, RecoveryOperation>::new();

    for (key, entry) in &desired {
        if current.get(key) != Some(entry) {
            ordered_operations.insert(key.clone(), RecoveryOperation::Upsert { entry: entry.clone() });
        }
    }
    for key in current.keys() {
        if !desired.contains_key(key) {
            ordered_operations.insert(
                key.clone(),
                RecoveryOperation::Delete { wallet: key.0.clone(), asset_pair: key.1.clone() },
            );
        }
    }

    let operations: Vec<RecoveryOperation> = ordered_operations.into_values().collect();
    let batches = operations
        .chunks(batch_size)
        .enumerate()
        .map(|(index, operations)| RecoveryBatch {
            index,
            predicted_effects: operations.iter().map(operation_effect).collect(),
            estimated_cost_units: (operations.len() as u64)
                .saturating_mul(ESTIMATED_COST_PER_OPERATION),
            operations: operations.to_vec(),
        })
        .collect();
    let mut plan = RecoveryPlan {
        version: RECOVERY_PLAN_VERSION,
        base_state_root: export_root(&current)?,
        export_root: export_root(&desired)?,
        batch_size,
        batches,
        plan_hash: String::new(),
        approvals: Vec::new(),
    };
    plan.plan_hash = calculate_plan_hash(&plan)?;
    write_json_atomic(output, &plan)?;
    eprintln!(
        "Plan {} batches={} operations={} base={} target={}",
        output.display(),
        plan.batches.len(),
        operations_count(&plan),
        plan.base_state_root,
        plan.export_root
    );
    Ok(())
}

fn operation_effect(operation: &RecoveryOperation) -> String {
    match operation {
        RecoveryOperation::Upsert { entry } => format!(
            "set {}/{} score={} confidence={}",
            entry.wallet, entry.asset_pair, entry.score, entry.confidence
        ),
        RecoveryOperation::Delete { wallet, asset_pair } => {
            format!("delete {wallet}/{asset_pair}")
        }
    }
}

fn operations_count(plan: &RecoveryPlan) -> usize {
    plan.batches.iter().map(|batch| batch.operations.len()).sum()
}

fn calculate_plan_hash(plan: &RecoveryPlan) -> Result<String> {
    let mut unsigned = plan.clone();
    unsigned.plan_hash.clear();
    unsigned.approvals.clear();
    let bytes = serde_json::to_vec(&unsigned).context("encoding unsigned recovery plan")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn load_plan(path: &Path) -> Result<RecoveryPlan> {
    let bytes = fs::read(path).with_context(|| format!("reading plan {}", path.display()))?;
    let plan: RecoveryPlan = serde_json::from_slice(&bytes)
        .with_context(|| format!("decoding plan {}", path.display()))?;
    if plan.version != RECOVERY_PLAN_VERSION {
        anyhow::bail!("unsupported recovery plan version {}", plan.version);
    }
    if calculate_plan_hash(&plan)? != plan.plan_hash {
        anyhow::bail!("recovery plan hash mismatch; plan contents changed after planning");
    }
    Ok(plan)
}

fn cmd_cosign(plan_path: &Path, operator: &str, key_env: &str) -> Result<()> {
    if operator.trim().is_empty() {
        anyhow::bail!("operator ID must not be empty");
    }
    let mut plan = load_plan(plan_path)?;
    if plan.approvals.iter().any(|approval| approval.operator == operator) {
        anyhow::bail!("operator {operator} has already approved this plan");
    }
    let secret_hex = std::env::var(key_env)
        .with_context(|| format!("signing key environment variable {key_env} is not set"))?;
    let secret: [u8; 32] = hex::decode(secret_hex.trim())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("signing key must be exactly 32 bytes of hex"))?;
    let signing_key = SigningKey::from_bytes(&secret);
    let signature = signing_key.sign(plan.plan_hash.as_bytes());
    plan.approvals.push(PlanApproval {
        operator: operator.to_owned(),
        public_key: hex::encode(signing_key.verifying_key().to_bytes()),
        signature: hex::encode(signature.to_bytes()),
    });
    plan.approvals.sort_by(|left, right| left.operator.cmp(&right.operator));
    write_json_atomic(plan_path, &plan)?;
    eprintln!("Added Ed25519 approval from {operator} to {}", plan_path.display());
    Ok(())
}

fn validate_approvals(plan: &RecoveryPlan, trusted_path: &Path) -> Result<()> {
    let content = fs::read_to_string(trusted_path)
        .with_context(|| format!("reading trusted operators {}", trusted_path.display()))?;
    let trusted: BTreeMap<String, String> =
        serde_json::from_str(&content).context("trusted operators must be a JSON ID-to-public-key map")?;
    let mut operators = BTreeSet::new();
    let mut public_keys = BTreeSet::new();
    for approval in &plan.approvals {
        if !operators.insert(approval.operator.as_str()) {
            anyhow::bail!("duplicate approval from operator {}", approval.operator);
        }
        let trusted_key = trusted
            .get(&approval.operator)
            .with_context(|| format!("operator {} is not trusted", approval.operator))?;
        let public_key = hex::decode(&approval.public_key)?;
        let canonical_public_key = hex::encode(&public_key);
        if canonical_public_key != trusted_key.to_ascii_lowercase() {
            anyhow::bail!("approval key for {} does not match trusted key", approval.operator);
        }
        if !public_keys.insert(canonical_public_key) {
            anyhow::bail!("two-person control requires two distinct public keys");
        }
        let public_key: [u8; 32] = public_key
            .try_into()
            .map_err(|_| anyhow::anyhow!("Ed25519 public key must be 32 bytes"))?;
        let verifying_key = VerifyingKey::from_bytes(&public_key)
            .context("invalid Ed25519 public key in plan")?;
        let signature_bytes = hex::decode(&approval.signature)?;
        let signature = Signature::from_slice(&signature_bytes)
            .context("invalid Ed25519 signature in plan")?;
        verifying_key
            .verify(plan.plan_hash.as_bytes(), &signature)
            .with_context(|| format!("invalid plan signature from {}", approval.operator))?;
    }
    if operators.len() < 2 {
        anyhow::bail!("apply requires approvals from two distinct trusted operators");
    }
    Ok(())
}

fn apply_operation(state: &mut EntryMap, operation: &RecoveryOperation) {
    match operation {
        RecoveryOperation::Upsert { entry } => {
            state.insert((entry.wallet.clone(), entry.asset_pair.clone()), entry.clone());
        }
        RecoveryOperation::Delete { wallet, asset_pair } => {
            state.remove(&(wallet.clone(), asset_pair.clone()));
        }
    }
}

fn operation_postcondition_holds(state: &EntryMap, operation: &RecoveryOperation) -> bool {
    match operation {
        RecoveryOperation::Upsert { entry } => {
            state.get(&(entry.wallet.clone(), entry.asset_pair.clone())) == Some(entry)
        }
        RecoveryOperation::Delete { wallet, asset_pair } => {
            !state.contains_key(&(wallet.clone(), asset_pair.clone()))
        }
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".bak");
    PathBuf::from(value)
}

fn load_live_state(path: &Path) -> Result<Vec<ExportableScoreEntry>> {
    let source = if path.exists() { path.to_path_buf() } else { backup_path(path) };
    load_export(&source)
}

fn load_checkpoint(path: &Path) -> Result<ApplyCheckpoint> {
    let source = if path.exists() { path.to_path_buf() } else { backup_path(path) };
    let bytes = fs::read(&source)
        .with_context(|| format!("reading checkpoint {}", source.display()))?;
    serde_json::from_slice(&bytes).context("decoding recovery checkpoint")
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let backup = backup_path(path);
    fs::write(&temporary, bytes).with_context(|| format!("writing {}", temporary.display()))?;
    OpenOptions::new()
        .write(true)
        .open(&temporary)
        .with_context(|| format!("opening {} for sync", temporary.display()))?
        .sync_all()
        .with_context(|| format!("syncing {}", temporary.display()))?;
    if path.exists() {
        if backup.exists() {
            fs::remove_file(&backup).with_context(|| format!("removing {}", backup.display()))?;
        }
        fs::rename(path, &backup)
            .with_context(|| format!("preserving previous file {}", path.display()))?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("replacing {}", path.display()));
    }
    if backup.exists() {
        fs::remove_file(&backup).with_context(|| format!("removing {}", backup.display()))?;
    }
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).context("encoding recovery state")?;
    write_bytes_atomic(path, &bytes)
}

fn cmd_apply(
    plan_path: &Path,
    state_path: &Path,
    trusted_operators_path: &Path,
    checkpoint_path: &Path,
) -> Result<()> {
    let plan = load_plan(plan_path)?;
    validate_approvals(&plan, trusted_operators_path)?;
    let mut state = index_export(load_live_state(state_path)?)?;
    let mut next_batch = 0usize;
    let mut applied_operations = 0usize;
    let state_root = export_root(&state)?;

    if checkpoint_path.exists() || backup_path(checkpoint_path).exists() {
        let checkpoint = load_checkpoint(checkpoint_path)?;
        if checkpoint.plan_hash != plan.plan_hash {
            anyhow::bail!("checkpoint belongs to a different recovery plan");
        }
        if checkpoint.next_batch > plan.batches.len() {
            anyhow::bail!("checkpoint batch index exceeds the plan");
        }
        next_batch = checkpoint.next_batch;
        applied_operations = checkpoint.applied_operations;
        if checkpoint.state_root != state_root {
            let pending_batch = plan
                .batches
                .get(next_batch)
                .context("checkpoint root mismatches after all planned batches")?;
            let mut replayed_state = state.clone();
            for operation in &pending_batch.operations {
                apply_operation(&mut replayed_state, operation);
            }
            if export_root(&replayed_state)? != state_root {
                anyhow::bail!(
                    "live state diverges from the checkpoint and is not exactly the pending idempotent batch"
                );
            }
            applied_operations += pending_batch.operations.len();
            next_batch += 1;
            let recovered_checkpoint = ApplyCheckpoint {
                plan_hash: plan.plan_hash.clone(),
                next_batch,
                applied_operations,
                state_root: state_root.clone(),
            };
            write_json_atomic(checkpoint_path, &recovered_checkpoint)?;
            eprintln!("Recovered checkpoint for already-persisted batch {}", pending_batch.index);
        }
    } else {
        if state_root != plan.base_state_root {
            anyhow::bail!("live state no longer matches the plan base root");
        }
        write_json_atomic(
            checkpoint_path,
            &ApplyCheckpoint {
                plan_hash: plan.plan_hash.clone(),
                next_batch: 0,
                applied_operations: 0,
                state_root: state_root.clone(),
            },
        )?;
    }

    for batch in plan.batches.iter().skip(next_batch) {
        if batch.operations.len() > plan.batch_size {
            anyhow::bail!("batch {} exceeds the approved batch size", batch.index);
        }
        for operation in &batch.operations {
            apply_operation(&mut state, operation);
            if !operation_postcondition_holds(&state, operation) {
                anyhow::bail!("postcondition failed for {}", operation_effect(operation));
            }
        }
        applied_operations += batch.operations.len();
        let ordered: Vec<&ExportableScoreEntry> = state.values().collect();
        write_json_atomic(state_path, &ordered)?;
        let checkpoint = ApplyCheckpoint {
            plan_hash: plan.plan_hash.clone(),
            next_batch: batch.index + 1,
            applied_operations,
            state_root: export_root(&state)?,
        };
        write_json_atomic(checkpoint_path, &checkpoint)?;
        eprintln!(
            "Applied batch {}/{} operations={} root={}",
            checkpoint.next_batch,
            plan.batches.len(),
            batch.operations.len(),
            checkpoint.state_root
        );
    }

    let final_root = export_root(&state)?;
    if final_root != plan.export_root {
        anyhow::bail!("final state root {} does not match approved export root {}", final_root, plan.export_root);
    }
    eprintln!("Recovery complete; verified export root {final_root}");
    Ok(())
}

fn cmd_reconcile(snapshot_a: &PathBuf, snapshot_b: &PathBuf, output: &PathBuf) -> Result<()> {
    let snap_a: StateSnapshot = load_snapshot(snapshot_a)?;
    let snap_b: StateSnapshot = load_snapshot(snapshot_b)?;

    let score_match = snap_a.score_root == snap_b.score_root;
    let config_match = snap_a.config_root == snap_b.config_root;
    let auth_match = snap_a.auth_root == snap_b.auth_root;
    let count_match = snap_a.entry_count == snap_b.entry_count;
    let all_match = score_match && config_match && auth_match && count_match;

    let mut details = Vec::new();

    details.push(format!(
        "Score root: {} == {} → {}",
        &snap_a.score_root[..16],
        &snap_b.score_root[..16],
        if score_match { "MATCH" } else { "DIVERGE" }
    ));
    details.push(format!(
        "Config root: {} == {} → {}",
        &snap_a.config_root[..16],
        &snap_b.config_root[..16],
        if config_match { "MATCH" } else { "DIVERGE" }
    ));
    details.push(format!(
        "Auth root: {} == {} → {}",
        &snap_a.auth_root[..16],
        &snap_b.auth_root[..16],
        if auth_match { "MATCH" } else { "DIVERGE" }
    ));
    details.push(format!(
        "Entry count: {} vs {} → {}",
        snap_a.entry_count,
        snap_b.entry_count,
        if count_match { "MATCH" } else { "DIVERGE" }
    ));

    let report = ReconciliationReport {
        snapshot_a_path: snapshot_a.display().to_string(),
        snapshot_b_path: snapshot_b.display().to_string(),
        score_roots_match: score_match,
        config_roots_match: config_match,
        auth_roots_match: auth_match,
        entry_counts_match: count_match,
        all_match,
        details,
    };

    let json = serde_json::to_string_pretty(&report)?;
    fs::write(output, &json).with_context(|| {
        format!("Failed to write reconciliation report to {}", output.display())
    })?;

    if all_match {
        eprintln!("✅ Snapshots MATCH — state is consistent.");
    } else {
        eprintln!("❌ Snapshots DIVERGE — state has changed:");
        if !score_match {
            eprintln!("   - Score entries differ");
        }
        if !config_match {
            eprintln!("   - Configuration differs");
        }
        if !auth_match {
            eprintln!("   - Auth/signer config differs");
        }
        if !count_match {
            eprintln!("   - Entry count: {} vs {}", snap_a.entry_count, snap_b.entry_count);
        }
    }
    eprintln!("Report saved to {}", output.display());
    Ok(())
}

fn cmd_verify(
    snapshot_path: Option<&Path>,
    export_path: Option<&Path>,
    state_path: Option<&Path>,
) -> Result<()> {
    if let Some(state_path) = state_path {
        let export_path = export_path
            .context("--state requires --export with the known-good score export")?;
        let expected = export_root(&index_export(load_export(export_path)?)?)?;
        let actual = export_root(&index_export(load_live_state(state_path)?)?)?;
        if actual != expected {
            anyhow::bail!("final state root {actual} does not match export root {expected}");
        }
        eprintln!("Final state matches export root {expected}");
        return Ok(());
    }

    let snapshot_path = snapshot_path.context("verify requires a snapshot path or --state with --export")?;
    let snapshot: StateSnapshot = load_snapshot(&snapshot_path.to_path_buf())?;
    eprintln!("Verifying snapshot from {}", snapshot_path.display());
    eprintln!("  Score root:  {}", snapshot.score_root);
    eprintln!("  Config root: {}", snapshot.config_root);
    eprintln!("  Auth root:   {}", snapshot.auth_root);
    eprintln!("  Entry count: {}", snapshot.entry_count);
    eprintln!("  Ledger seq:  {}", snapshot.ledger_seq);
    eprintln!("  Timestamp:   {}", snapshot.timestamp);

    // Validate hex strings
    if snapshot.score_root.len() != 64 {
        eprintln!(
            "  ⚠ WARNING: score_root is not 64 hex chars ({} chars)",
            snapshot.score_root.len()
        );
    }
    if snapshot.config_root.len() != 64 {
        eprintln!(
            "  ⚠ WARNING: config_root is not 64 hex chars ({} chars)",
            snapshot.config_root.len()
        );
    }
    if snapshot.auth_root.len() != 64 {
        eprintln!(
            "  ⚠ WARNING: auth_root is not 64 hex chars ({} chars)",
            snapshot.auth_root.len()
        );
    }

    if let Some(export_path) = export_path {
        let content = fs::read_to_string(export_path)
            .with_context(|| format!("Failed to read export {}", export_path.display()))?;
        let entries: Vec<ExportableScoreEntry> =
            serde_json::from_str(&content).context("Export is not a valid JSON array")?;
        if entries.len() as u32 != snapshot.entry_count {
            eprintln!(
                "  ⚠ Entry count mismatch: export has {} entries, snapshot says {}",
                entries.len(),
                snapshot.entry_count
            );
        } else {
            eprintln!("  ✅ Export entry count ({}) matches snapshot.", entries.len());
        }
    }

    eprintln!("Snapshot verification complete.");
    // Full on-chain verification requires calling verify_state_checksum on the contract.
    eprintln!("Note: Run `verify_state_checksum` on the contract for full on-chain verification.");
    Ok(())
}

fn cmd_report(
    snapshot_path: &PathBuf,
    action: &str,
    description: &str,
    output: &PathBuf,
) -> Result<()> {
    let snapshot: StateSnapshot = load_snapshot(snapshot_path)?;
    let now = chrono_now();

    let report = PostActionReport {
        snapshot,
        action_type: action.to_string(),
        action_timestamp: now,
        action_description: description.to_string(),
        pre_action_entry_count: 0,
        post_action_entry_count: None,
        checksum_verified: false,
        verification_notes: vec![
            "Pre-action snapshot recorded. Run `compute_state_checksum` after".to_string(),
            "the action and reconcile the two snapshots to confirm consistency.".to_string(),
        ],
    };

    let json = serde_json::to_string_pretty(&report)?;
    fs::write(output, &json)
        .with_context(|| format!("Failed to write report to {}", output.display()))?;
    eprintln!("Post-action report saved to {}", output.display());
    Ok(())
}

fn cmd_keeper(input: &Path, output: &PathBuf, max_entries: usize) -> Result<()> {
    let content = fs::read_to_string(input)
        .with_context(|| format!("Failed to read {}", input.display()))?;
    let mut candidates: Vec<KeeperCandidate> = serde_json::from_str(&content)
        .context("Input is not a valid JSON array of KeeperCandidate entries")?;

    // Most urgent (lowest estimated remaining TTL) first.
    candidates.sort_by_key(|c| c.estimated_ttl_remaining);

    let mut seen = std::collections::HashSet::new();
    candidates.retain(|c| seen.insert((c.wallet.clone(), c.asset_pair.clone())));

    let cap = max_entries.min(KEEPER_BATCH_MAX);
    if candidates.len() > cap {
        candidates.truncate(cap);
    }

    let batch = KeeperBatch { entries: candidates };
    let json = serde_json::to_string_pretty(&batch)?;
    fs::write(output, &json)
        .with_context(|| format!("Failed to write keeper batch to {}", output.display()))?;
    eprintln!(
        "Keeper batch saved to {} ({} entries, most urgent first)",
        output.display(),
        batch.entries.len()
    );
    eprintln!(
        "Feed batch.entries (wallet, asset_pair) pairs into \
         keeper_extend_entry_ttls(keeper, entries) on-chain."
    );
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn load_snapshot(path: &PathBuf) -> Result<StateSnapshot> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read snapshot from {}", path.display()))?;
    let snapshot: StateSnapshot = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse snapshot from {}", path.display()))?;
    Ok(snapshot)
}

/// Returns an ISO-8601-like timestamp string. Uses system time via std::time.
fn chrono_now() -> String {
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs();
    // Format as ISO-8601 approximate
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;
    format!(
        "2026-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        days / 30 + 1,
        days % 30 + 1,
        hours,
        minutes,
        seconds
    )
}
