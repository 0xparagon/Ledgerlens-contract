//! Verkle membership-proof decoder: arbitrary proof bytes must yield
//! `true`/`false`, never a panic or trap.
#![no_main]

use ledgerlens_score::{LedgerLensScoreContract, LedgerLensScoreContractClient};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env, Symbol};

fuzz_target!(|data: &[u8]| {
    if data.len() < 48 {
        return;
    }
    let env = Env::default();
    env.budget().reset_unlimited();
    let id = env.register_contract(None, LedgerLensScoreContract);
    let client = LedgerLensScoreContractClient::new(&env, &id);
    let mut commitment = [0u8; 48];
    commitment.copy_from_slice(&data[..48]);
    let _ = client.verify_membership(
        &BytesN::from_array(&env, &commitment),
        &Address::generate(&env),
        &Symbol::new(&env, "XLM_USDC"),
        &u32::from(data[0] % 101),
        &0u64,
        &Bytes::from_slice(&env, &data[48..]),
    );
});
