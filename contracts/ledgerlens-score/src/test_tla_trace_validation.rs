use std::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec as StdVec,
};

use serde::Deserialize;
use serde_json::Value;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger as _},
    Address, Bytes, Env, Vec,
};

use crate::{
    constants::MIN_UPGRADE_DELAY_SECS, storage, LedgerLensScoreContract,
    LedgerLensScoreContractClient,
};

const BASE_TIMESTAMP: u64 = 1_700_000_000;
const SECONDS_PER_TLA_TICK: u64 = MIN_UPGRADE_DELAY_SECS;
const REPLAY_MAPPING: &str = include_str!("../../../spec/refinement-replay-map.json");

#[derive(Debug, Deserialize)]
struct Behavior {
    behavior_id: String,
    states: StdVec<TraceState>,
}

#[derive(Debug, Deserialize)]
struct TraceState {
    step: usize,
    action: String,
    arguments: StdVec<Value>,
    variables: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct ReplayMapping {
    model_constants: BTreeMap<String, u64>,
    abstract_state: BTreeMap<String, String>,
    actions: BTreeMap<String, Value>,
}

#[derive(Debug)]
struct TraceFrame {
    behavior_id: String,
    step: usize,
    action: String,
    expected: BTreeMap<String, String>,
    actual: BTreeMap<String, String>,
}

fn read_generated_behaviors() -> StdVec<Behavior> {
    let trace_path = std::env::var("LEDGERLENS_TLA_TRACE_FILE")
        .expect("run tools/validate_tla_trace_replay.py to generate TLC behaviors");
    serde_json::from_str(&std::fs::read_to_string(trace_path).unwrap()).unwrap()
}

fn model_number(state: &TraceState, name: &str) -> u64 {
    state.variables[name].as_u64().unwrap()
}

fn model_bool(state: &TraceState, name: &str) -> bool {
    state.variables[name].as_bool().unwrap()
}

fn model_map_number(state: &TraceState, name: &str, key: &str) -> u64 {
    state.variables[name][key].as_u64().unwrap()
}

fn model_refill_count(state: &TraceState, wallet: &str, cooldown: u64) -> u64 {
    let elapsed = model_number(state, "now") - model_map_number(state, "tb_last_refill", wallet);
    (model_map_number(state, "tb_tokens", wallet) + elapsed / cooldown)
        .min(model_number(state, "tb_capacity"))
}

fn model_signers(state: &TraceState) -> StdVec<String> {
    let mut signers = state.variables["upg_live_signers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|signer| signer.as_str().unwrap().to_string())
        .collect::<StdVec<_>>();
    signers.sort();
    signers
}

fn model_projection(state: &TraceState, cooldown: u64) -> BTreeMap<String, String> {
    let mut projection = BTreeMap::new();
    for variable in ["score", "hwm", "breach_count", "last_submit_time"] {
        for wallet in ["W1", "W2"] {
            projection.insert(
                format!("{variable}.{wallet}"),
                model_map_number(state, variable, wallet).to_string(),
            );
        }
    }
    projection.insert("tb_capacity".to_string(), model_number(state, "tb_capacity").to_string());
    for wallet in ["W1", "W2"] {
        projection.insert(
            format!("effective_tokens.{wallet}"),
            model_refill_count(state, wallet, cooldown).to_string(),
        );
    }
    projection.insert("now".to_string(), model_number(state, "now").to_string());
    projection.insert("paused".to_string(), model_bool(state, "replay_paused").to_string());
    let proposal_open = model_number(state, "gov_proposal_id") != 0
        && !model_bool(state, "gov_vetoed")
        && !model_bool(state, "gov_executed");
    projection.insert("upgrade_pending".to_string(), proposal_open.to_string());
    projection.insert(
        "proposal_created_at".to_string(),
        if proposal_open { model_number(state, "gov_proposed_at") } else { 0 }.to_string(),
    );
    projection
        .insert("upgrade_executed".to_string(), model_bool(state, "gov_executed").to_string());
    projection
        .insert("admin_signers".to_string(), serde_json::to_string(&model_signers(state)).unwrap());
    projection
}

fn rust_timestamp(model_now: u64) -> u64 {
    BASE_TIMESTAMP + (model_now - 1) * SECONDS_PER_TLA_TICK
}

fn abstract_contract_state(
    env: &Env,
    contract_id: &Address,
    wallets: &[Address; 2],
    signers: &[Address; 3],
    pair: &soroban_sdk::Symbol,
    mapping: &ReplayMapping,
    upgrade_executed: bool,
) -> BTreeMap<String, String> {
    env.as_contract(contract_id, || {
        let mut projection = BTreeMap::new();
        for (model_variable, rust_mapping) in &mapping.abstract_state {
            match rust_mapping.as_str() {
                "storage::get_score" => {
                    for (index, wallet) in ["W1", "W2"].iter().enumerate() {
                        let value = storage::get_score(env, &wallets[index], pair)
                            .map(|record| record.score)
                            .unwrap_or(0);
                        projection.insert(format!("{model_variable}.{wallet}"), value.to_string());
                    }
                }
                "storage::get_historical_max_score" => {
                    for (index, wallet) in ["W1", "W2"].iter().enumerate() {
                        let value = storage::get_historical_max_score(env, &wallets[index], pair);
                        projection.insert(format!("{model_variable}.{wallet}"), value.to_string());
                    }
                }
                "storage::get_breach_count" => {
                    for (index, wallet) in ["W1", "W2"].iter().enumerate() {
                        let value = storage::get_breach_count(env, &wallets[index], pair);
                        projection.insert(format!("{model_variable}.{wallet}"), value.to_string());
                    }
                }
                "storage::get_last_submit_time" => {
                    for (index, wallet) in ["W1", "W2"].iter().enumerate() {
                        let timestamp = storage::get_last_submit_time(env, &wallets[index], pair);
                        let value = if timestamp == 0 {
                            0
                        } else {
                            (timestamp - BASE_TIMESTAMP) / SECONDS_PER_TLA_TICK + 1
                        };
                        projection.insert(format!("{model_variable}.{wallet}"), value.to_string());
                    }
                }
                "storage::get_burst_capacity" => {
                    projection.insert(
                        model_variable.clone(),
                        storage::get_burst_capacity(env).to_string(),
                    );
                }
                "storage::get_token_bucket/storage::get_last_submit_time/storage::get_pair_cooldown_secs" => {
                    let capacity = storage::get_burst_capacity(env);
                    let cooldown = storage::get_pair_cooldown_secs(env, pair).max(1);
                    for (index, wallet) in ["W1", "W2"].iter().enumerate() {
                        let last_submit = storage::get_last_submit_time(env, &wallets[index], pair);
                        let available = if capacity <= 1 {
                            if last_submit == 0
                                || env.ledger().timestamp().saturating_sub(last_submit) >= cooldown
                            {
                                capacity
                            } else {
                                0
                            }
                        } else {
                            match storage::get_token_bucket(env, &wallets[index], pair) {
                                Some(bucket) => {
                                    let elapsed = env.ledger().timestamp().saturating_sub(bucket.last_refill);
                                    (bucket.tokens as u64 + elapsed / cooldown).min(capacity as u64) as u32
                                }
                                None => capacity,
                            }
                        };
                        projection.insert(format!("effective_tokens.{wallet}"), available.to_string());
                    }
                }
                "Env::ledger::timestamp" => {
                    let value =
                        (env.ledger().timestamp() - BASE_TIMESTAMP) / SECONDS_PER_TLA_TICK + 1;
                    projection.insert(model_variable.clone(), value.to_string());
                }
                "storage::is_paused" => {
                    projection.insert("paused".to_string(), storage::is_paused(env).to_string());
                }
                "storage::get_pending_upgrade" if model_variable == "GovProposalOpen" => {
                    projection.insert(
                        "upgrade_pending".to_string(),
                        storage::get_pending_upgrade(env).is_some().to_string(),
                    );
                }
                "storage::get_pending_upgrade" if model_variable == "gov_proposed_at" => {
                    let created_at = storage::get_pending_upgrade(env)
                        .map(|proposal| {
                            (proposal.proposed_at - BASE_TIMESTAMP) / SECONDS_PER_TLA_TICK + 1
                        })
                        .unwrap_or(0);
                    projection.insert("proposal_created_at".to_string(), created_at.to_string());
                }
                "execute_upgrade" => {
                    projection.insert("upgrade_executed".to_string(), upgrade_executed.to_string());
                }
                "storage::get_admin_set" => {
                    let live_signers = storage::get_admin_set(env);
                    let mut labels = StdVec::new();
                    for index in 0..live_signers.len() {
                        let address = live_signers.get(index).unwrap();
                        let label = signers
                            .iter()
                            .position(|candidate| candidate == &address)
                            .map(|index| ["S1", "S2", "S3"][index].to_string())
                            .expect("contract signer set contains an unmapped address");
                        labels.push(label);
                    }
                    labels.sort();
                    projection.insert(
                        "admin_signers".to_string(),
                        serde_json::to_string(&labels).unwrap(),
                    );
                }
                entry => panic!("unimplemented Rust abstraction mapping {entry}"),
            }
        }
        projection
    })
}

fn validate_replayed_transition(frame: &TraceFrame) -> Result<(), String> {
    if frame.expected == frame.actual {
        return Ok(());
    }
    Err(format!(
        "behavior_id={} step={} action={} expected={} actual={}",
        frame.behavior_id,
        frame.step,
        frame.action,
        serde_json::to_string(&frame.expected).unwrap(),
        serde_json::to_string(&frame.actual).unwrap(),
    ))
}

fn auth_signers(env: &Env, state: &TraceState, signers: &[Address; 3]) -> Vec<Address> {
    let mut result = Vec::new(env);
    for signer in model_signers(state) {
        let index = match signer.as_str() {
            "S1" => 0,
            "S2" => 1,
            "S3" => 2,
            _ => unreachable!(),
        };
        result.push_back(signers[index].clone());
    }
    result
}

fn assert_state_matches(
    behavior: &Behavior,
    state: &TraceState,
    env: &Env,
    contract_id: &Address,
    wallets: &[Address; 2],
    signers: &[Address; 3],
    pair: &soroban_sdk::Symbol,
    upgrade_executed: bool,
    cooldown: u64,
) -> Result<(), String> {
    let frame = TraceFrame {
        behavior_id: behavior.behavior_id.clone(),
        step: state.step,
        action: state.action.clone(),
        expected: model_projection(state, cooldown),
        actual: abstract_contract_state(
            env,
            contract_id,
            wallets,
            signers,
            pair,
            &serde_json::from_str(REPLAY_MAPPING).unwrap(),
            upgrade_executed,
        ),
    };
    validate_replayed_transition(&frame)
}

fn replay_behavior(behavior: &Behavior, mutate_submit_translation: bool) -> Result<(), String> {
    let mapping: ReplayMapping = serde_json::from_str(REPLAY_MAPPING).unwrap();
    let cooldown = mapping.model_constants["COOLDOWN"];
    assert!(cooldown > 0);
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| ledger.timestamp = BASE_TIMESTAMP);

    let contract_id = env.register_contract(None, LedgerLensScoreContract);
    let client = LedgerLensScoreContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    let wallets = [Address::generate(&env), Address::generate(&env)];
    let signers = [Address::generate(&env), Address::generate(&env), Address::generate(&env)];
    let pair = symbol_short!("XLM_USDC");

    client.initialize(&admin, &service);
    for signer in &signers {
        client.add_admin_signer(&Vec::new(&env), signer);
    }
    client.set_admin_threshold(&Vec::new(&env), &2);
    env.as_contract(&contract_id, || {
        storage::set_risk_threshold(&env, mapping.model_constants["RISK_THRESHOLD"] as u32)
    });
    let wasm_hash = env.deployer().upload_contract_wasm(Bytes::new(&env));
    let mut upgrade_executed = false;

    let initial = behavior.states.first().expect("TLC behavior has no initial state");
    assert_state_matches(
        behavior,
        initial,
        &env,
        &contract_id,
        &wallets,
        &signers,
        &pair,
        upgrade_executed,
        cooldown,
    )?;

    for (index, state) in behavior.states.iter().enumerate().skip(1) {
        let previous = &behavior.states[index - 1];
        let action_mapping = mapping.actions.get(&state.action).unwrap_or_else(|| {
            panic!("TLC emitted unmapped action {} in {}", state.action, behavior.behavior_id)
        });
        let entry_point = action_mapping["rust_entrypoint"].as_str().unwrap();
        let timestamp = rust_timestamp(model_number(state, "now"));
        env.ledger().with_mut(|ledger| ledger.timestamp = timestamp);
        let signer_auth = auth_signers(&env, previous, &signers);

        match entry_point {
            "Env::ledger::timestamp" | "stutter" => {}
            "submit_score" => {
                let wallet_label = state.arguments[0].as_str().unwrap();
                let wallet_index = if wallet_label == "W1" { 0 } else { 1 };
                let model_score = state.arguments[1].as_u64().unwrap() as u32;
                let submitted_score = model_score + u32::from(mutate_submit_translation);
                client.submit_score(
                    &Vec::new(&env),
                    &wallets[wallet_index],
                    &pair,
                    &submitted_score,
                    &false,
                    &false,
                    &timestamp,
                    &90,
                    &1,
                    &None,
                );
            }
            "pause" => client.pause(&signer_auth),
            "unpause" => client.unpause(&signer_auth),
            "add_admin_signer/remove_admin_signer" => {
                let label = state.arguments[0].as_str().unwrap();
                let signer_index = match label {
                    "S1" => 0,
                    "S2" => 1,
                    "S3" => 2,
                    _ => unreachable!(),
                };
                if model_signers(previous).iter().any(|active| active == label) {
                    client.remove_admin_signer(&signer_auth, &signers[signer_index]);
                } else {
                    client.add_admin_signer(&signer_auth, &signers[signer_index]);
                }
            }
            "propose_upgrade" => client.propose_upgrade(&signer_auth, &wasm_hash),
            "execute_upgrade" => {
                client.execute_upgrade(&signer_auth);
                upgrade_executed = true;
            }
            entry_point => panic!("unimplemented Rust trace mapping {entry_point}"),
        }

        assert_state_matches(
            behavior,
            state,
            &env,
            &contract_id,
            &wallets,
            &signers,
            &pair,
            upgrade_executed,
            cooldown,
        )?;
    }
    Ok(())
}

#[test]
#[ignore = "requires generated TLC traces; run tools/validate_tla_trace_replay.py"]
fn test_tla_trace_validation_replays_generated_behaviors() {
    let behaviors = read_generated_behaviors();
    let mut covered = BTreeMap::<String, bool>::new();
    for behavior in &behaviors {
        for state in behavior.states.iter().skip(1) {
            covered.insert(state.action.clone(), true);
        }
        replay_behavior(behavior, false).unwrap_or_else(|error| panic!("{error}"));
    }
    for required in [
        "SubmitScore",
        "PauseContract",
        "UnpauseContract",
        "MutateAdminSet",
        "ProposeGov",
        "ExecuteGov",
    ] {
        assert!(covered.contains_key(required), "TLC behavior set missed {required}");
    }
}

#[test]
#[ignore = "requires generated TLC traces; run tools/validate_tla_trace_replay.py"]
fn test_tla_trace_validator_detects_mutated_contract_invocation() {
    let behaviors = read_generated_behaviors();
    let behavior = behaviors
        .iter()
        .find(|behavior| behavior.states.iter().any(|state| state.action == "SubmitScore"))
        .expect("generated TLC behaviors contain no SubmitScore action");
    let error = replay_behavior(behavior, true).unwrap_err();
    std::println!("{error}");
    assert!(error.contains(&format!("behavior_id={}", behavior.behavior_id)));
    assert!(error.contains(" step="));
    assert!(error.contains("action=SubmitScore"));
    assert!(error.contains("expected="));
    assert!(error.contains("actual="));
}
