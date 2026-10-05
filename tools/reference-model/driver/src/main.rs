//! Differential-testing driver (issue #1240).
//!
//! Reads one JSON array of operations per stdin line, executes the sequence
//! against a fresh `ledgerlens-score` instance, and writes one JSON array of
//! observable results per stdout line. The independent Python model in
//! `tools/reference-model/ledgerlens_ref.py` produces the same shape; the
//! runner diffs the two. Errors are reported as their discriminant.

use std::io::{self, BufRead, Write};

use ledgerlens_score::{LedgerLensScoreContract, LedgerLensScoreContractClient};
use serde::Deserialize;
use serde_json::{json, Value};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env, Symbol, Vec};

const WALLETS: usize = 3;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Op {
    Advance { secs: u64 },
    Submit { w: usize, p: String, score: u32, conf: u32 },
    Weight { p: String, weight: u32 },
    Decay { num: u64, den: u64 },
    Floor { v: u32 },
    Get { w: usize, p: String },
    Eff { w: usize, p: String },
    Agg { w: usize },
    Gate { w: usize, p: String, t: u32, q: u32 },
}

fn err(code: u32) -> Value {
    json!({ "err": code })
}

fn run(ops: &[Op]) -> std::vec::Vec<Value> {
    let env = Env::default();
    env.mock_all_auths();
    env.budget().reset_unlimited();
    env.ledger().with_mut(|l| l.timestamp = 1_700_000_000);
    let id = env.register_contract(None, LedgerLensScoreContract);
    let c = LedgerLensScoreContractClient::new(&env, &id);
    c.initialize(&Address::generate(&env), &Address::generate(&env));
    let wallets: std::vec::Vec<Address> = (0..WALLETS).map(|_| Address::generate(&env)).collect();
    let sym = |p: &str| Symbol::new(&env, p);
    let none = Vec::<Address>::new(&env);

    ops.iter()
        .map(|op| match op {
            Op::Advance { secs } => {
                env.ledger().with_mut(|l| l.timestamp += secs);
                Value::Null
            }
            Op::Submit { w, p, score, conf } => {
                let ts = env.ledger().timestamp();
                match c.try_submit_score(
                    &none,
                    &wallets[*w],
                    &sym(p),
                    score,
                    &false,
                    &false,
                    &ts,
                    conf,
                    &1,
                    &None,
                ) {
                    Ok(_) => json!("ok"),
                    Err(Ok(e)) => err(e as u32),
                    Err(Err(_)) => json!("trap"),
                }
            }
            Op::Weight { p, weight } => match c.try_set_pair_weight(&none, &sym(p), weight) {
                Ok(_) => json!("ok"),
                Err(Ok(e)) => err(e as u32),
                Err(Err(_)) => json!("trap"),
            },
            Op::Decay { num, den } => match c.try_set_decay_rate(num, den) {
                Ok(_) => json!("ok"),
                Err(Ok(e)) => err(e as u32),
                Err(Err(_)) => json!("trap"),
            },
            Op::Floor { v } => match c.try_set_global_min_confidence(v) {
                Ok(_) => json!("ok"),
                Err(Ok(e)) => err(e as u32),
                Err(Err(_)) => json!("trap"),
            },
            Op::Get { w, p } => match c.try_get_score(&wallets[*w], &sym(p)) {
                Ok(Ok(s)) => json!([s.score, s.confidence]),
                Err(Ok(e)) => err(e as u32),
                _ => json!("trap"),
            },
            Op::Eff { w, p } => match c.try_get_effective_score(&wallets[*w], &sym(p)) {
                Ok(Ok(s)) => json!(s.effective_score),
                Err(Ok(e)) => err(e as u32),
                _ => json!("trap"),
            },
            Op::Agg { w } => match c.try_get_aggregate_score(&wallets[*w]) {
                Ok(Ok(a)) => json!([a.aggregate_score, a.pair_count, a.max_pair_score]),
                Err(Ok(e)) => err(e as u32),
                _ => json!("trap"),
            },
            Op::Gate { w, p, t, q } => {
                match c.try_query_risk_gate_with_confidence(&wallets[*w], &sym(p), t, q) {
                    Ok(Ok(b)) => json!(b),
                    _ => json!("trap"),
                }
            }
        })
        .collect()
}

fn main() {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in io::stdin().lock().lines() {
        let ops: std::vec::Vec<Op> =
            serde_json::from_str(&line.expect("stdin")).expect("valid op sequence");
        writeln!(out, "{}", Value::Array(run(&ops))).expect("stdout");
    }
}
