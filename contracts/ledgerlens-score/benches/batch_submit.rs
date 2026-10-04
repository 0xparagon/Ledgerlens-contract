//! Criterion benchmarks for `submit_scores_batch` throughput at varying batch sizes.
//!
//! Run: `cargo bench -p ledgerlens-score --bench batch_submit`
//!
//! Batch sizes above [`MAX_BATCH`] are submitted as multiple contract calls
//! (ceil(n / MAX_BATCH)) because on-chain `MAX_BATCH_SIZE` is 20.
//!
//! Issue #1160 adds selectable atomicity modes. This bench measures the cost of
//! the cheap pre-validation pass that all-or-nothing mode performs before any
//! write, by running identical inputs through best-effort and all-or-nothing
//! modes side by side.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use ledgerlens_score::{
    AtomicityMode, LedgerLensScoreContract, LedgerLensScoreContractClient, ScoreSubmission,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, Symbol, Vec,
};

const MAX_BATCH: u32 = 20;

fn setup(env: &Env) -> (LedgerLensScoreContractClient<'_>, Symbol) {
    env.mock_all_auths();
    env.budget().reset_unlimited();
    env.ledger().with_mut(|l| l.timestamp = 1_700_000_000);

    let contract_id = env.register_contract(None, LedgerLensScoreContract);
    let client = LedgerLensScoreContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    let service = Address::generate(env);
    client.initialize(&admin, &service);

    let asset_pair = Symbol::new(env, "XLM_USDC");
    (client, asset_pair)
}

fn build_entries(
    env: &Env,
    asset_pair: &Symbol,
    count: u32,
    batch_index: u32,
) -> Vec<ScoreSubmission> {
    let mut batch = Vec::new(env);
    for i in 0..count {
        let wallet = Address::generate(env);
        batch.push_back(ScoreSubmission {
            wallet,
            asset_pair: asset_pair.clone(),
            score: 30 + batch_index + i,
            benford_flag: false,
            ml_flag: false,
            timestamp: 1_700_000_000 + batch_index as u64,
            confidence: 90,
            model_version: 1,
        });
    }
    batch
}

fn submit_n_entries(
    env: &Env,
    client: &LedgerLensScoreContractClient,
    asset_pair: &Symbol,
    total: u32,
    mode: AtomicityMode,
) -> (u64, u64) {
    // Benchmark large, multi-call batches without treating the host's
    // single-transaction default ceiling as a harness-wide limit.
    env.budget().reset_unlimited();
    env.budget().reset_tracker();

    let mut remaining = total;
    let mut batch_index = 0u32;
    while remaining > 0 {
        let chunk = remaining.min(MAX_BATCH);
        let batch = build_entries(env, asset_pair, chunk, batch_index);
        black_box(client.submit_scores_batch_with_mode(&batch, &mode));
        remaining -= chunk;
        batch_index += 1;
        env.ledger().with_mut(|l| l.timestamp += 3_601);
    }

    (env.budget().cpu_instruction_cost(), env.budget().memory_bytes_cost())
}

fn bench_batch_submit(c: &mut Criterion) {
    let mut group = c.benchmark_group("submit_scores_batch");
    group.sample_size(10);

    for size in [1u32, 10, 50, 100] {
        group.bench_with_input(BenchmarkId::new("entries", size), &size, |b, &size| {
            b.iter(|| {
                let env = Env::default();
                let (client, asset_pair) = setup(&env);
                black_box(submit_n_entries(
                    &env,
                    &client,
                    &asset_pair,
                    size,
                    AtomicityMode::BestEffort,
                ))
            });
        });
    }

    group.finish();
}

/// Measures the extra cost of the all-or-nothing pre-validation pass relative
/// to best-effort on identical inputs (issue #1160 acceptance criteria).
fn bench_atomicity_modes(c: &mut Criterion) {
    let mut group = c.benchmark_group("submit_scores_batch_atomicity");
    group.sample_size(10);

    for size in [1u32, 10, 50, 100] {
        for mode in [AtomicityMode::BestEffort, AtomicityMode::AllOrNothing] {
            group.bench_with_input(
                BenchmarkId::new(format!("{:?}", mode), size),
                &size,
                |b, &size| {
                    b.iter(|| {
                        let env = Env::default();
                        let (client, asset_pair) = setup(&env);
                        black_box(submit_n_entries(&env, &client, &asset_pair, size, mode))
                    });
                },
            );
        }
    }

    group.finish();
}

criterion_group!(benches, bench_batch_submit, bench_atomicity_modes);
criterion_main!(benches);
