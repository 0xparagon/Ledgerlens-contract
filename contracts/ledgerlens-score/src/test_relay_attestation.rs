//! Tests for the permissionless attested-relay path (`relay_attested_score`).
//!
//! Signatures are produced with a real secp256k1 key (via the `k256` test
//! dependency, the same approach `test_attestation.rs` uses for
//! `submit_score`'s legacy attestation), so these exercise
//! `compute_relay_commitment` / `verify_signature` end-to-end.

use k256::ecdsa::SigningKey;
use soroban_sdk::{
    symbol_short, testutils::Address as _, Address, Bytes, BytesN, Env, Symbol, Vec,
};

use crate::{Error, LedgerLensScoreContract, LedgerLensScoreContractClient, RelayScoreAttestation};

fn initialized<'a>() -> (Env, LedgerLensScoreContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, LedgerLensScoreContract);
    let client = LedgerLensScoreContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    client.initialize(&admin, &service);
    (env, client, admin, service)
}

fn signing_key(seed: u8) -> SigningKey {
    let mut bytes = [0u8; 32];
    bytes[31] = seed;
    bytes[0] = 1;
    SigningKey::from_bytes((&bytes).into()).unwrap()
}

fn pubkey_bytes(env: &Env, key: &SigningKey) -> Bytes {
    let point = key.verifying_key().to_encoded_point(true);
    Bytes::from_slice(env, point.as_bytes())
}

fn get_contract_id_bytes(env: &Env, contract_address: &Address) -> BytesN<32> {
    let xdr = contract_address.to_xdr(env);
    let mut bytes = [0u8; 32];
    if xdr.len() >= 32 {
        bytes.copy_from_slice(&xdr.as_ref()[..32]);
    }
    BytesN::from_array(env, &bytes)
}

#[allow(clippy::too_many_arguments)]
fn relay_digest(
    env: &Env,
    contract_addr: &Address,
    wallet: &Address,
    pair: &Symbol,
    score: u32,
    benford_flag: bool,
    ml_flag: bool,
    timestamp: u64,
    confidence: u32,
    model_version: u32,
    unsigned: &RelayScoreAttestation,
) -> [u8; 32] {
    env.as_contract(contract_addr, || {
        LedgerLensScoreContract::compute_relay_commitment(
            env,
            wallet,
            pair,
            score,
            benford_flag,
            ml_flag,
            timestamp,
            confidence,
            model_version,
            unsigned,
        )
        .unwrap()
        .to_array()
    })
}

fn sign(key: &SigningKey, digest: [u8; 32]) -> [u8; 65] {
    let (sig, recid) = key.sign_prehash_recoverable(&digest).expect("sign failed");
    let mut sig_bytes = [0u8; 65];
    sig_bytes[..64].copy_from_slice(&sig.to_bytes());
    sig_bytes[64] = recid.to_byte();
    sig_bytes
}

/// Builds a fully valid, signed `RelayScoreAttestation` for the given
/// payload and nonce/expiry.
#[allow(clippy::too_many_arguments)]
fn build_attestation(
    env: &Env,
    client: &LedgerLensScoreContractClient,
    key: &SigningKey,
    wallet: &Address,
    pair: &Symbol,
    score: u32,
    nonce: u64,
    valid_before_ledger: u32,
) -> RelayScoreAttestation {
    let contract_version = client.get_contract_version();
    let contract_id = get_contract_id_bytes(env, &client.address);
    let unsigned = RelayScoreAttestation {
        commitment: BytesN::from_array(env, &[0u8; 32]),
        signature: BytesN::from_array(env, &[0u8; 65]),
        contract_id: contract_id.clone(),
        contract_version,
        nonce,
        valid_before_ledger,
    };
    let digest =
        relay_digest(env, &client.address, wallet, pair, score, false, false, 1, 90, 1, &unsigned);
    let sig = sign(key, digest);
    RelayScoreAttestation {
        commitment: BytesN::from_array(env, &digest),
        signature: BytesN::from_array(env, &sig),
        contract_id,
        contract_version,
        nonce,
        valid_before_ledger,
    }
}

// ── happy path ───────────────────────────────────────────────────────────────

#[test]
fn relay_accepted_writes_score_and_advances_nonce() {
    let (env, client, _admin, _service) = initialized();
    let key = signing_key(1);
    client.set_service_pubkey(&Vec::new(&env), &pubkey_bytes(&env, &key));

    let wallet = Address::generate(&env);
    let pair = symbol_short!("XLM_USDC");
    let relayer = Address::generate(&env);
    let att = build_attestation(&env, &client, &key, &wallet, &pair, 42, 0, 1_000_000);

    let accepted = client.relay_attested_score(
        &relayer, &wallet, &pair, &42, &false, &false, &1, &90, &1, &att,
    );
    assert!(accepted);
    assert_eq!(client.get_score(&wallet, &pair).score, 42);
}

// ── duplicate relay: cheap, well-defined no-op ────────────────────────────────

#[test]
fn duplicate_relay_is_a_noop_independent_of_which_relayer_posts_first() {
    let (env, client, _admin, _service) = initialized();
    let key = signing_key(1);
    client.set_service_pubkey(&Vec::new(&env), &pubkey_bytes(&env, &key));

    let wallet = Address::generate(&env);
    let pair = symbol_short!("XLM_USDC");
    let relayer_a = Address::generate(&env);
    let relayer_b = Address::generate(&env);
    let att = build_attestation(&env, &client, &key, &wallet, &pair, 55, 0, 1_000_000);

    let first =
        client.relay_attested_score(&relayer_a, &wallet, &pair, &55, &false, &false, &1, &90, &1, &att);
    assert!(first);
    let score_after_first = client.get_score(&wallet, &pair);

    // A second relayer posts the *same* attestation.
    let second =
        client.relay_attested_score(&relayer_b, &wallet, &pair, &55, &false, &false, &1, &90, &1, &att);
    assert!(!second);
    // Resulting state is unchanged — independent of which relayer posted it.
    assert_eq!(client.get_score(&wallet, &pair), score_after_first);
}

#[test]
fn out_of_order_relays_of_different_nonces_both_apply_in_arrival_order() {
    let (env, client, _admin, _service) = initialized();
    let key = signing_key(1);
    client.set_service_pubkey(&Vec::new(&env), &pubkey_bytes(&env, &key));

    let wallet = Address::generate(&env);
    let pair = symbol_short!("XLM_USDC");
    let relayer = Address::generate(&env);

    let att0 = build_attestation(&env, &client, &key, &wallet, &pair, 10, 0, 1_000_000);
    let att1 = build_attestation(&env, &client, &key, &wallet, &pair, 20, 1, 1_000_000);

    // Relayer holds both and submits nonce 0 then nonce 1 — the only valid
    // order, since nonce 1 isn't valid until nonce 0 has been consumed.
    assert!(client.relay_attested_score(
        &relayer, &wallet, &pair, &10, &false, &false, &1, &90, &1, &att0
    ));
    assert!(client.relay_attested_score(
        &relayer, &wallet, &pair, &20, &false, &false, &1, &90, &1, &att1
    ));
    assert_eq!(client.get_score(&wallet, &pair).score, 20);

    // Submitting nonce 1 again (already consumed) is rejected, not replayed.
    assert_eq!(
        client.try_relay_attested_score(&relayer, &wallet, &pair, &20, &false, &false, &1, &90, &1, &att1),
        Err(Ok(Error::InvalidAttestation))
    );
}

// ── stale attestation ────────────────────────────────────────────────────────

#[test]
fn stale_attestation_past_valid_before_ledger_rejected() {
    let (env, client, _admin, _service) = initialized();
    let key = signing_key(1);
    client.set_service_pubkey(&Vec::new(&env), &pubkey_bytes(&env, &key));

    let wallet = Address::generate(&env);
    let pair = symbol_short!("XLM_USDC");
    let relayer = Address::generate(&env);
    // valid_before_ledger is already in the past relative to the ledger the
    // call executes on (the sandboxed test env starts at sequence 0 by
    // default but advances at least implicitly via env internals — force a
    // deterministic past bound instead).
    let att = build_attestation(&env, &client, &key, &wallet, &pair, 10, 0, 0);
    use soroban_sdk::testutils::Ledger as _;
    env.ledger().set_sequence_number(5);

    assert_eq!(
        client.try_relay_attested_score(&relayer, &wallet, &pair, &10, &false, &false, &1, &90, &1, &att),
        Err(Ok(Error::StaleAttestation))
    );
}

// ── relay after key revocation ────────────────────────────────────────────────

#[test]
fn relay_after_service_pubkey_rotation_with_old_key_rejected() {
    let (env, client, _admin, _service) = initialized();
    let old_key = signing_key(1);
    let new_key = signing_key(2);
    client.set_service_pubkey(&Vec::new(&env), &pubkey_bytes(&env, &old_key));

    let wallet = Address::generate(&env);
    let pair = symbol_short!("XLM_USDC");
    let relayer = Address::generate(&env);
    // Attestation signed with the key that is about to be revoked.
    let att = build_attestation(&env, &client, &old_key, &wallet, &pair, 10, 0, 1_000_000);

    // Rotate immediately to the new key with no overlap window, revoking
    // the old key's authority.
    client.set_service_pubkey(&Vec::new(&env), &pubkey_bytes(&env, &new_key));

    assert_eq!(
        client.try_relay_attested_score(&relayer, &wallet, &pair, &10, &false, &false, &1, &90, &1, &att),
        Err(Ok(Error::InvalidAttestation))
    );
}

// ── nonce binding (fixes the legacy ScoreAttestation gap for this path) ──────

#[test]
fn signature_cannot_be_replayed_against_a_different_nonce() {
    let (env, client, _admin, _service) = initialized();
    let key = signing_key(1);
    client.set_service_pubkey(&Vec::new(&env), &pubkey_bytes(&env, &key));

    let wallet = Address::generate(&env);
    let pair = symbol_short!("XLM_USDC");
    let relayer = Address::generate(&env);

    // Build a validly-signed attestation for nonce 0, then hand-tamper the
    // nonce field to 5 without re-signing — nonce is part of the signed
    // digest for this attestation format, so the signature no longer
    // verifies against the recomputed commitment.
    let mut att = build_attestation(&env, &client, &key, &wallet, &pair, 10, 0, 1_000_000);
    att.nonce = 5;

    assert_eq!(
        client.try_relay_attested_score(&relayer, &wallet, &pair, &10, &false, &false, &1, &90, &1, &att),
        Err(Ok(Error::InvalidAttestation))
    );
}
