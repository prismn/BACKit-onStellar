#![cfg(test)]
extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env,
};

use crate::DutchAuction;
use crate::DutchAuctionClient;

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let outcome_manager = Address::generate(&env);
    let caller = Address::generate(&env);
    (env, admin, outcome_manager, caller)
}

fn default_ledger(ts: u64) -> LedgerInfo {
    LedgerInfo {
        protocol_version: 23,
        sequence_number: 1,
        timestamp: ts,
        network_id: [0u8; 32],
        base_reserve: 10,
        min_temp_entry_ttl: 0,
        min_persistent_entry_ttl: 0,
        max_entry_ttl: 0,
    }
}

fn init_contract<'a>(
    env: &'a Env,
    admin: &'a Address,
    outcome_manager: &'a Address,
) -> DutchAuctionClient<'a> {
    let contract_id = env.register(DutchAuction, ());
    let client = DutchAuctionClient::new(env, &contract_id);
    client.initialize(admin, outcome_manager, &3600, &86400, &100);
    client
}

#[test]
fn test_initialize_success() {
    let (env, admin, outcome_manager, _) = setup();
    let client = init_contract(&env, &admin, &outcome_manager);
    let info = client.get_auction_info(&1);
    assert_eq!(info, None);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn test_double_initialize_fails() {
    let (env, admin, outcome_manager, _) = setup();
    let contract_id = env.register(DutchAuction, ());
    let client = DutchAuctionClient::new(&env, &contract_id);
    client.initialize(&admin, &outcome_manager, &3600, &86400, &100);
    client.initialize(&admin, &outcome_manager, &3600, &86400, &100);
}

#[test]
fn test_start_and_get_auction_info() {
    let (env, admin, outcome_manager, caller) = setup();
    let client = init_contract(&env, &admin, &outcome_manager);
    env.ledger().set(default_ledger(1_000_000));
    client.start_dutch_auction(&1, &1000_0000000, &1);
    let info = client.get_auction_info(&1).unwrap();
    assert_eq!(info.call_id, 1);
    assert_eq!(info.condition_type, 1);
    assert_eq!(info.start_price, 1000_0000000);
    assert_eq!(info.settled, false);
}

#[test]
fn test_price_decay_target_above() {
    let (env, admin, outcome_manager, _) = setup();
    let client = init_contract(&env, &admin, &outcome_manager);
    env.ledger().set(default_ledger(1_000_000));
    let start_price: i128 = 1000_0000000;
    client.start_dutch_auction(&1, &start_price, &1);
    let price_at_start = client.get_dutch_auction_price(&1);
    assert_eq!(price_at_start, start_price * 2);

    env.ledger().set(default_ledger(1_001_800));
    let price_mid = client.get_dutch_auction_price(&1);
    assert_eq!(price_mid, start_price * 2 * (3600 - 1800) / 3600);

    env.ledger().set(default_ledger(1_003_600));
    let price_end = client.get_dutch_auction_price(&1);
    assert_eq!(price_end, 0);
}

#[test]
fn test_price_decay_target_below() {
    let (env, admin, outcome_manager, _) = setup();
    let client = init_contract(&env, &admin, &outcome_manager);
    env.ledger().set(default_ledger(1_000_000));
    let start_price: i128 = 1000_0000000;
    client.start_dutch_auction(&1, &start_price, &2);
    let price_at_start = client.get_dutch_auction_price(&1);
    assert_eq!(price_at_start, start_price / 2);

    env.ledger().set(default_ledger(1_001_800));
    let price_mid = client.get_dutch_auction_price(&1);
    let numerator = start_price * (3600i128 + 3 * 1800i128);
    let expected_mid = numerator / (2 * 3600i128);
    assert_eq!(price_mid, expected_mid);

    env.ledger().set(default_ledger(1_003_600));
    let price_end = client.get_dutch_auction_price(&1);
    assert_eq!(price_end, start_price * 2);
}

#[test]
fn test_settle_auction() {
    let (env, admin, outcome_manager, caller) = setup();
    let client = init_contract(&env, &admin, &outcome_manager);
    env.ledger().set(default_ledger(1_000_000));
    client.start_dutch_auction(&1, &1000_0000000, &1);
    env.ledger().set(default_ledger(1_001_800));
    let price = client.settle_dutch_auction(&caller, &1);
    let expected_price = 1000_0000000 * 2 * (3600 - 1800) / 3600;
    assert_eq!(price, expected_price);
    let info = client.get_auction_info(&1).unwrap();
    assert_eq!(info.settled, true);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_double_settle_fails() {
    let (env, admin, outcome_manager, caller) = setup();
    let client = init_contract(&env, &admin, &outcome_manager);
    env.ledger().set(default_ledger(1_000_000));
    client.start_dutch_auction(&1, &1000_0000000, &1);
    env.ledger().set(default_ledger(1_001_800));
    client.settle_dutch_auction(&caller, &1);
    client.settle_dutch_auction(&caller, &1);
}

#[test]
fn test_update_params() {
    let (env, admin, outcome_manager, _) = setup();
    let client = init_contract(&env, &admin, &outcome_manager);
    client.update_params(&7200, &43200, &200);
    env.ledger().set(default_ledger(1_000_000));
    client.start_dutch_auction(&1, &1000_0000000, &1);
    env.ledger().set(default_ledger(1_003_600));
    let price_mid = client.get_dutch_auction_price(&1);
    let expected = 1000_0000000 * 2 * (7200 - 3600) / 7200;
    assert_eq!(price_mid, expected);
}

#[test]
fn test_settle_at_end_price_target_above() {
    let (env, admin, outcome_manager, caller) = setup();
    let client = init_contract(&env, &admin, &outcome_manager);
    env.ledger().set(default_ledger(1_000_000));
    client.start_dutch_auction(&1, &500_0000000, &1);
    env.ledger().set(default_ledger(1_003_600));
    let price = client.settle_dutch_auction(&caller, &1);
    assert_eq!(price, 0);
}

#[test]
fn test_start_with_invalid_price_fails() {
    let (env, admin, outcome_manager, _) = setup();
    let client = init_contract(&env, &admin, &outcome_manager);
    env.ledger().set(default_ledger(1_000_000));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.start_dutch_auction(&1, &0, &1);
    }));
    assert!(result.is_err());
}

#[test]
fn test_settle_unstarted_auction_fails() {
    let (env, admin, outcome_manager, caller) = setup();
    let client = init_contract(&env, &admin, &outcome_manager);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.settle_dutch_auction(&caller, &42);
    }));
    assert!(result.is_err());
}

// ─── Falling-price asset sales ────────────────────────────────────────────

mod sale_tests {
    use super::*;
    use crate::sale::{SaleError, CURVE_LINEAR, CURVE_QUADRATIC};
    use soroban_sdk::{
        testutils::Events as _,
        token::{Client as TokenClient, StellarAssetClient},
        IntoVal, TryFromVal, Val, Vec,
    };

    struct Sale<'a> {
        env: Env,
        client: DutchAuctionClient<'a>,
        sell: TokenClient<'a>,
        pay: TokenClient<'a>,
        seller: Address,
        buyer: Address,
    }

    fn setup_sale<'a>() -> Sale<'a> {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set(default_ledger(1_000));
        let admin = Address::generate(&env);
        let sell_sac = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let pay_sac = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let seller = Address::generate(&env);
        let buyer = Address::generate(&env);
        StellarAssetClient::new(&env, &sell_sac).mint(&seller, &500);
        StellarAssetClient::new(&env, &pay_sac).mint(&buyer, &10_000);
        let client = DutchAuctionClient::new(&env, &env.register(DutchAuction, ()));
        Sale {
            sell: TokenClient::new(&env, &sell_sac),
            pay: TokenClient::new(&env, &pay_sac),
            env,
            client,
            seller,
            buyer,
        }
    }

    #[test]
    fn linear_price_decays_to_the_floor() {
        let s = setup_sale();
        let id = s.client.create_auction(
            &s.seller,
            &s.sell.address,
            &s.pay.address,
            &500,
            &1_000,
            &200,
            &800,
            &CURVE_LINEAR,
        );
        assert_eq!(s.client.get_sale_price(&id), 1_000);
        s.env.ledger().set(default_ledger(1_200));
        assert_eq!(s.client.get_sale_price(&id), 800); // 1000 - 800 * 200 / 800
        s.env.ledger().set(default_ledger(1_400));
        assert_eq!(s.client.get_sale_price(&id), 600);
        s.env.ledger().set(default_ledger(9_999));
        assert_eq!(s.client.get_sale_price(&id), 200);
    }

    #[test]
    fn quadratic_price_decays_faster_early() {
        let s = setup_sale();
        let id = s.client.create_auction(
            &s.seller,
            &s.sell.address,
            &s.pay.address,
            &500,
            &1_000,
            &200,
            &800,
            &CURVE_QUADRATIC,
        );
        s.env.ledger().set(default_ledger(1_400));
        // 200 + 800 * (400/800)^2 = 400 (linear would be 600)
        assert_eq!(s.client.get_sale_price(&id), 400);
    }

    #[test]
    fn buy_pays_the_exact_price_swaps_the_lot_and_emits_auction_filled() {
        let s = setup_sale();
        let id = s.client.create_auction(
            &s.seller,
            &s.sell.address,
            &s.pay.address,
            &500,
            &1_000,
            &200,
            &800,
            &CURVE_LINEAR,
        );
        s.env.ledger().set(default_ledger(1_400));

        assert_eq!(
            s.client.try_buy(&s.buyer, &id, &500),
            Err(Ok(SaleError::PriceAboveMax))
        );
        assert_eq!(s.client.buy(&s.buyer, &id, &700), 600);
        let events = s.env.events().all();

        assert_eq!(s.pay.balance(&s.buyer), 10_000 - 600);
        assert_eq!(s.pay.balance(&s.seller), 600);
        assert_eq!(s.sell.balance(&s.buyer), 500);

        let topics: Vec<Val> = ("dutch_auction", "AuctionFilled").into_val(&s.env);
        let event = events
            .iter()
            .find(|(c, t, _)| *c == s.client.address && *t == topics)
            .expect("AuctionFilled not emitted");
        let (auction_id, buyer, price) =
            <(u64, Address, i128)>::try_from_val(&s.env, &event.2).unwrap();
        assert_eq!((auction_id, buyer, price), (id, s.buyer.clone(), 600));

        assert_eq!(
            s.client.try_buy(&s.buyer, &id, &700),
            Err(Ok(SaleError::AuctionClosed))
        );
    }

    #[test]
    fn seller_can_cancel_an_unsold_auction() {
        let s = setup_sale();
        let id = s.client.create_auction(
            &s.seller,
            &s.sell.address,
            &s.pay.address,
            &500,
            &1_000,
            &200,
            &800,
            &CURVE_LINEAR,
        );
        assert_eq!(s.sell.balance(&s.seller), 0);
        s.client.cancel_auction(&id);
        assert_eq!(s.sell.balance(&s.seller), 500);
        assert_eq!(
            s.client.try_cancel_auction(&id),
            Err(Ok(SaleError::AuctionClosed))
        );
    }

    #[test]
    fn invalid_parameters_are_rejected() {
        let s = setup_sale();
        for (amount, start, floor, duration) in [
            (0, 1_000, 200, 800),
            (500, 200, 200, 800),
            (500, 1_000, 200, 0),
        ] {
            assert_eq!(
                s.client.try_create_auction(
                    &s.seller,
                    &s.sell.address,
                    &s.pay.address,
                    &amount,
                    &start,
                    &floor,
                    &duration,
                    &CURVE_LINEAR,
                ),
                Err(Ok(SaleError::InvalidParams))
            );
        }
    }
}
