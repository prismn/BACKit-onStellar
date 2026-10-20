#![allow(deprecated)]

use soroban_sdk::{Address, Env};

pub fn emit_dutch_auction_started(
    env: &Env,
    call_id: u64,
    start_price: i128,
    condition_type: u32,
    start_ts: u64,
) {
    env.events().publish(
        ("dutch_auction", "started"),
        (call_id, start_price, condition_type, start_ts),
    );
}

pub fn emit_dutch_auction_settled(
    env: &Env,
    call_id: u64,
    price: i128,
    settler: &Address,
    reward: i128,
) {
    env.events().publish(
        ("dutch_auction", "settled"),
        (call_id, price, settler.clone(), reward),
    );
}

pub fn emit_auth_params_changed(
    env: &Env,
    auction_duration_secs: u64,
    oracle_deadline_secs: u64,
    settler_reward_bps: u32,
) {
    env.events().publish(
        ("dutch_auction", "params_changed"),
        (
            auction_duration_secs,
            oracle_deadline_secs,
            settler_reward_bps,
        ),
    );
}

/// `AuctionFilled(auction_id, buyer, final_price)`.
pub fn emit_auction_filled(env: &Env, auction_id: u64, buyer: &Address, final_price: i128) {
    env.events().publish(
        ("dutch_auction", "AuctionFilled"),
        (auction_id, buyer.clone(), final_price),
    );
}
