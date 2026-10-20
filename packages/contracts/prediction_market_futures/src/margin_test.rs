#![cfg(test)]

use crate::margin::{liquidation_price, required_margin, MarginError};
use crate::{PredictionMarketFutures, PredictionMarketFuturesClient};
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Ctx<'a> {
    client: PredictionMarketFuturesClient<'a>,
    token: TokenClient<'a>,
    user: Address,
    liquidator: Address,
}

fn setup<'a>(env: &'a Env) -> Ctx<'a> {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let sac = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let user = Address::generate(env);
    StellarAssetClient::new(env, &sac).mint(&user, &1_000_000);

    let client =
        PredictionMarketFuturesClient::new(env, &env.register(PredictionMarketFutures, ()));
    client.initialize(&Address::generate(env));
    client.set_mark_price(&1u64, &100_000);

    Ctx {
        client,
        token: TokenClient::new(env, &sac),
        user,
        liquidator: Address::generate(env),
    }
}

#[test]
fn margin_and_liquidation_price_math() {
    assert_eq!(required_margin(1_000, 5), Ok(200));
    assert_eq!(required_margin(1_001, 5), Ok(201));
    assert_eq!(required_margin(0, 5), Err(MarginError::InvalidSize));
    assert_eq!(required_margin(10, 11), Err(MarginError::InvalidLeverage));
    // 5x long: 100_000 * (1 - 0.2 + 0.05) = 85_000; short: 100_000 * (1 + 0.2 - 0.05) = 115_000
    assert_eq!(liquidation_price(100_000, 5, true), Ok(85_000));
    assert_eq!(liquidation_price(100_000, 5, false), Ok(115_000));
}

#[test]
fn long_is_liquidated_when_mark_crosses_with_five_percent_bounty() {
    let env = Env::default();
    let ctx = setup(&env);

    let id = ctx
        .client
        .open_position(&ctx.user, &1u64, &ctx.token.address, &10_000, &5, &true);
    let position = ctx.client.get_margin_position(&id);
    assert_eq!(
        (position.margin, position.liquidation_price),
        (2_000, 85_000)
    );
    assert_eq!(ctx.token.balance(&ctx.user), 998_000);

    // Not yet liquidatable.
    ctx.client.set_mark_price(&1u64, &90_000);
    assert_eq!(
        ctx.client.try_liquidate(&ctx.liquidator, &id),
        Err(Ok(MarginError::NotLiquidatable))
    );

    // Mark crosses 85_000: loss = 10_000 * 15% = 1_500, remaining 500, bounty = min(5% of 2_000 = 100, 500).
    ctx.client.set_mark_price(&1u64, &85_000);
    assert_eq!(ctx.client.liquidate(&ctx.liquidator, &id), 100);
    assert_eq!(ctx.token.balance(&ctx.liquidator), 100);
    assert_eq!(ctx.token.balance(&ctx.user), 998_000 + 400);
    assert!(!ctx.client.get_margin_position(&id).open);
    assert_eq!(
        ctx.client.try_liquidate(&ctx.liquidator, &id),
        Err(Ok(MarginError::PositionClosed))
    );
}

#[test]
fn loss_is_capped_at_margin_so_the_position_never_goes_insolvent() {
    let env = Env::default();
    let ctx = setup(&env);
    let id = ctx
        .client
        .open_position(&ctx.user, &1u64, &ctx.token.address, &10_000, &5, &false);

    // Short against a 3x price spike: loss would exceed margin; payout is zero, never negative.
    ctx.client.set_mark_price(&1u64, &300_000);
    assert_eq!(ctx.client.liquidate(&ctx.liquidator, &id), 0);
    assert_eq!(ctx.token.balance(&ctx.user), 998_000);
}
