#![cfg(test)]

extern crate std;

use crate::{PredictionMarketFactory, PredictionMarketFactoryClient};
use backit_shared::{build_message, OUTCOME_DOWN, OUTCOME_UP};
use outcome_manager::{OutcomeManager, OutcomeManagerClient, SignedOutcome};
use prediction_market::{ConditionType, MarketInitArgs, PredictionMarketClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Bytes, BytesN, Env, Vec,
};

fn install_market_wasm(env: &Env) -> BytesN<32> {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let release_v1 = workspace_root.join("target/wasm32v1-none/release");
    let release_unknown = workspace_root.join("target/wasm32-unknown-unknown/release");
    let candidates = [
        release_v1.join("prediction_market.optimized.wasm"),
        release_v1.join("prediction_market.wasm"),
        release_unknown.join("prediction_market.optimized.wasm"),
        release_unknown.join("prediction_market.wasm"),
    ];

    let wasm_path = candidates
        .iter()
        .find(|path| path.exists())
        .expect("missing prediction_market WASM");

    let wasm_bytes = std::fs::read(wasm_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", wasm_path.display()));
    env.deployer().upload_contract_wasm(wasm_bytes.as_slice())
}

fn default_market_args(env: &Env, stake_token: &Address, end_ts: u64) -> MarketInitArgs {
    MarketInitArgs {
        stake_token: stake_token.clone(),
        stake_amount: 1_000_000,
        start_price: 100_000_000,
        end_ts,
        token_address: stake_token.clone(),
        pair_id: Bytes::from_slice(env, b"XLM-USDC"),
        metadata_hash: BytesN::from_array(env, &[1u8; 32]),
        condition: ConditionType::PercentUp(5),
        outcome_count: 2,
    }
}

fn setup_token(env: &Env, admin: &Address) -> Address {
    let token = env.register_stellar_asset_contract_v2(admin.clone());
    let sac = token.address();
    let stellar = StellarAssetClient::new(env, &sac);
    stellar.mint(admin, &100_000_000_000);
    sac
}

fn gen_keypair(env: &Env) -> (BytesN<32>, BytesN<32>) {
    use ed25519_dalek::SigningKey;
    use rand::RngCore;

    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let signing_key = SigningKey::from_bytes(&seed);
    let public_key = signing_key.verifying_key();
    (
        BytesN::from_array(env, &seed),
        BytesN::from_array(env, &public_key.to_bytes()),
    )
}

fn sign_outcome(
    env: &Env,
    secret: &BytesN<32>,
    call_id: u64,
    outcome: u32,
    price: i128,
    timestamp: u64,
) -> BytesN<64> {
    use ed25519_dalek::{Signer, SigningKey};

    let msg = build_message(env, call_id, outcome, price, timestamp);
    let mut msg_bytes = [0u8; 128];
    let msg_len = msg.len() as usize;
    msg.copy_into_slice(&mut msg_bytes[..msg_len]);
    let signing_key = SigningKey::from_bytes(&secret.to_array());
    let signature = signing_key.sign(&msg_bytes[..msg_len]);
    BytesN::from_array(env, &signature.to_bytes())
}

struct TestSetup<'a> {
    env: Env,
    admin: Address,
    creator: Address,
    staker_a: Address,
    staker_b: Address,
    token: Address,
    factory: PredictionMarketFactoryClient<'a>,
    outcome_mgr: OutcomeManagerClient<'a>,
    oracle_secret: BytesN<32>,
    oracle_pubkey: BytesN<32>,
}

fn setup_full_stack<'a>() -> TestSetup<'a> {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let staker_a = Address::generate(&env);
    let staker_b = Address::generate(&env);
    let token = setup_token(&env, &admin);

    let market_wasm = install_market_wasm(&env);
    let factory_id = env.register(PredictionMarketFactory, ());
    let factory = PredictionMarketFactoryClient::new(&env, &factory_id);

    let (oracle_secret, oracle_pubkey) = gen_keypair(&env);
    let outcome_id = env.register(OutcomeManager, ());
    let outcome_mgr = OutcomeManagerClient::new(&env, &outcome_id);
    let fee_collector = Address::generate(&env);

    let mut oracles = Vec::new(&env);
    oracles.push_back(oracle_pubkey.clone());

    outcome_mgr.initialize(&admin, &oracles, &1u32, &fee_collector, &100u32, &0u64);
    factory.initialize(&admin, &outcome_id, &market_wasm, &100_000);
    factory.whitelist_token(&token);
    outcome_mgr.set_factory(&factory_id);

    TestSetup {
        env,
        admin,
        creator,
        staker_a,
        staker_b,
        token,
        factory,
        outcome_mgr,
        oracle_secret,
        oracle_pubkey,
    }
}

#[test]
fn factory_deploy_stake_resolve_payout() {
    let setup = setup_full_stack();
    let env = &setup.env;

    let end_ts = env.ledger().timestamp() + 3600;
    let args = default_market_args(env, &setup.token, end_ts);

    let market_addr = setup.factory.deploy_market(&setup.creator, &args);
    assert_eq!(setup.factory.get_market_count(), 1);

    let market = PredictionMarketClient::new(env, &market_addr);
    let call_id = market.get_call_id();

    let token = TokenClient::new(env, &setup.token);
    token.transfer(&setup.admin, &setup.staker_a, &5_000_000);
    token.transfer(&setup.admin, &setup.staker_b, &5_000_000);

    market.stake_on_call(&setup.staker_a, &call_id, &2_000_000, &OUTCOME_UP);
    market.stake_on_call(&setup.staker_b, &call_id, &3_000_000, &OUTCOME_DOWN);

    env.ledger().set_timestamp(end_ts + 1);

    let signed = SignedOutcome {
        call_id,
        outcome: OUTCOME_UP,
        price: 110_000_000,
        timestamp: end_ts + 1,
        oracle_pubkey: setup.oracle_pubkey.clone(),
        signature: sign_outcome(
            env,
            &setup.oracle_secret,
            call_id,
            OUTCOME_UP,
            110_000_000,
            end_ts + 1,
        ),
    };
    setup
        .outcome_mgr
        .submit_outcome_for_market(&signed, &end_ts);

    setup.outcome_mgr.mark_settled(&market_addr, &call_id);

    let total_winning = 2_000_000i128;
    let total_losing = 3_000_000i128;
    let balance_before = token.balance(&setup.staker_a);

    setup.outcome_mgr.claim_payout_for_market(
        &call_id,
        &setup.staker_a,
        &total_winning,
        &total_winning,
        &total_losing,
    );

    let balance_after = token.balance(&setup.staker_a);
    assert!(balance_after > balance_before);

    let call = market.get_call(&call_id);
    assert_eq!(call.outcome, OUTCOME_UP);
}

#[test]
fn multiple_concurrent_markets() {
    let setup = setup_full_stack();
    let env = &setup.env;
    let base_ts = env.ledger().timestamp() + 7200;

    let mut markets = Vec::new(env);
    for i in 0..3u32 {
        let end_ts = base_ts + i as u64 * 100;
        let args = default_market_args(env, &setup.token, end_ts);
        let addr = setup.factory.deploy_market(&setup.creator, &args);
        markets.push_back(addr);
    }

    assert_eq!(setup.factory.get_market_count(), 3);
    let page = setup.factory.get_all_markets(&0, &10);
    assert_eq!(page.len(), 3);

    for i in 0..3u32 {
        let market_addr = markets.get(i).unwrap();
        let market = PredictionMarketClient::new(env, &market_addr);
        let call_id = market.get_call_id();
        assert_eq!(setup.factory.get_market(&call_id), market_addr);
    }
}

#[test]
fn stress_test_twenty_markets() {
    let setup = setup_full_stack();
    let env = &setup.env;
    let base_ts = env.ledger().timestamp() + 10_000;

    for i in 0..20u64 {
        let end_ts = base_ts + i;
        let args = default_market_args(env, &setup.token, end_ts);
        setup.factory.deploy_market(&setup.creator, &args);
    }

    assert_eq!(setup.factory.get_market_count(), 20);

    let page_a = setup.factory.get_all_markets(&0, &10);
    let page_b = setup.factory.get_all_markets(&10, &10);
    assert_eq!(page_a.len(), 10);
    assert_eq!(page_b.len(), 10);

    for i in 1..=20u64 {
        assert!(setup.factory.try_get_market(&i).is_ok());
    }
}

#[test]
fn factory_pagination_bounds() {
    let setup = setup_full_stack();
    let env = &setup.env;
    let end_ts = env.ledger().timestamp() + 5000;

    for _ in 0..5 {
        let args = default_market_args(env, &setup.token, end_ts);
        setup.factory.deploy_market(&setup.creator, &args);
    }

    assert_eq!(setup.factory.get_all_markets(&100, &10).len(), 0);
    assert_eq!(setup.factory.get_all_markets(&3, &2).len(), 2);
}

#[test]
fn deployed_market_address_is_deterministic_and_matches_prediction() {
    let setup = setup_full_stack();
    let env = &setup.env;
    let end_ts = env.ledger().timestamp() + 3600;
    let args = default_market_args(env, &setup.token, end_ts);

    let predicted_first = setup.factory.predict_market_address(&1u64);
    assert_eq!(predicted_first, setup.factory.predict_market_address(&1u64));
    assert_ne!(predicted_first, setup.factory.predict_market_address(&2u64));

    let first = setup.factory.deploy_market(&setup.creator, &args);
    let second = setup.factory.deploy_market(&setup.creator, &args);
    assert_eq!(first, predicted_first);
    assert_eq!(second, setup.factory.predict_market_address(&2u64));
    assert_eq!(setup.factory.get_market(&1u64), first);
}

#[test]
fn factory_initialization_parameters_are_recorded_and_not_reinitialisable() {
    let setup = setup_full_stack();
    let config = setup.factory.get_config();
    assert_eq!(config.admin, setup.admin);
    assert!(!config.paused);
    assert!(setup
        .factory
        .try_initialize(
            &setup.admin,
            &config.outcome_manager,
            &config.market_wasm_hash,
            &1i128
        )
        .is_err());
}
