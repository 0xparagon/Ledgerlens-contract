//! Bulletproof range-proof decoder: arbitrary proof bytes must yield
//! `true`/`false`, never a panic or trap.
#![no_main]

use ledgerlens_score::{LedgerLensScoreContract, LedgerLensScoreContractClient};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env, Symbol};

fuzz_target!(|data: &[u8]| {
    if data.len() < 33 {
        return;
    }
    let env = Env::default();
    env.budget().reset_unlimited();
    let id = env.register_contract(None, LedgerLensScoreContract);
    let client = LedgerLensScoreContractClient::new(&env, &id);
    let mut commitment = [0u8; 32];
    commitment.copy_from_slice(&data[1..33]);
    let _ = client.verify_score_range_proof(
        &Address::generate(&env),
        &Symbol::new(&env, "XLM_USDC"),
        &BytesN::from_array(&env, &commitment),
        &Bytes::from_slice(&env, &data[33..]),
        &u32::from(data[0] % 101),
    );
});
