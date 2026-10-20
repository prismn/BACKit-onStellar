//! Falling-price asset auctions.
//!
//! A seller escrows `amount` of `sell_token` and the price decays from `start_price` to
//! `floor_price` over `duration` seconds:
//!
//! - linear:    `P(t) = start - (start - floor) * (t - t0) / T`
//! - quadratic: `P(t) = floor + (start - floor) * (T - (t - t0))^2 / T^2` (falls fast, then flattens)
//!
//! `buy` pays exactly the current price (so there is never an overpayment to refund) and fails if
//! the price is above the buyer's `max_price`.

use soroban_sdk::{contracterror, contracttype, token, Address, Env};

pub const CURVE_LINEAR: u32 = 0;
pub const CURVE_QUADRATIC: u32 = 1;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SaleError {
    InvalidParams = 101,
    AuctionNotFound = 102,
    AuctionClosed = 103,
    PriceAboveMax = 104,
    Overflow = 105,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SaleAuction {
    pub id: u64,
    pub seller: Address,
    pub sell_token: Address,
    pub pay_token: Address,
    pub amount: i128,
    pub start_price: i128,
    pub floor_price: i128,
    pub start_ts: u64,
    pub duration: u64,
    pub curve: u32,
    pub open: bool,
}

#[contracttype]
pub enum SaleKey {
    NextId,
    Auction(u64),
}

pub fn get_auction(env: &Env, id: u64) -> Option<SaleAuction> {
    env.storage().persistent().get(&SaleKey::Auction(id))
}

fn set_auction(env: &Env, auction: &SaleAuction) {
    env.storage()
        .persistent()
        .set(&SaleKey::Auction(auction.id), auction);
}

/// Exact price at `now`, clamped to `[floor_price, start_price]`.
pub fn price_at(auction: &SaleAuction, now: u64) -> Result<i128, SaleError> {
    let elapsed = now.saturating_sub(auction.start_ts).min(auction.duration);
    let span = auction.start_price - auction.floor_price;
    let duration = i128::from(auction.duration);
    if auction.curve == CURVE_QUADRATIC {
        let remaining = i128::from(auction.duration - elapsed);
        let scaled = span
            .checked_mul(
                remaining
                    .checked_mul(remaining)
                    .ok_or(SaleError::Overflow)?,
            )
            .ok_or(SaleError::Overflow)?
            / duration.checked_mul(duration).ok_or(SaleError::Overflow)?;
        Ok(auction.floor_price + scaled)
    } else {
        let decayed = span
            .checked_mul(i128::from(elapsed))
            .ok_or(SaleError::Overflow)?
            / duration;
        Ok(auction.start_price - decayed)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn create(
    env: &Env,
    seller: &Address,
    sell_token: &Address,
    pay_token: &Address,
    amount: i128,
    start_price: i128,
    floor_price: i128,
    duration: u64,
    curve: u32,
) -> Result<u64, SaleError> {
    if amount <= 0
        || floor_price < 0
        || start_price <= floor_price
        || duration == 0
        || curve > CURVE_QUADRATIC
    {
        return Err(SaleError::InvalidParams);
    }
    token::Client::new(env, sell_token).transfer(seller, &env.current_contract_address(), &amount);

    let id: u64 = env
        .storage()
        .persistent()
        .get(&SaleKey::NextId)
        .unwrap_or(0)
        + 1;
    env.storage().persistent().set(&SaleKey::NextId, &id);
    set_auction(
        env,
        &SaleAuction {
            id,
            seller: seller.clone(),
            sell_token: sell_token.clone(),
            pay_token: pay_token.clone(),
            amount,
            start_price,
            floor_price,
            start_ts: env.ledger().timestamp(),
            duration,
            curve,
            open: true,
        },
    );
    Ok(id)
}

/// Returns the final price paid.
pub fn buy(
    env: &Env,
    buyer: &Address,
    auction_id: u64,
    max_price: i128,
) -> Result<i128, SaleError> {
    let mut auction = get_auction(env, auction_id).ok_or(SaleError::AuctionNotFound)?;
    if !auction.open {
        return Err(SaleError::AuctionClosed);
    }
    let price = price_at(&auction, env.ledger().timestamp())?;
    if price > max_price {
        return Err(SaleError::PriceAboveMax);
    }

    token::Client::new(env, &auction.pay_token).transfer(buyer, &auction.seller, &price);
    token::Client::new(env, &auction.sell_token).transfer(
        &env.current_contract_address(),
        buyer,
        &auction.amount,
    );
    auction.open = false;
    set_auction(env, &auction);
    Ok(price)
}

/// Seller reclaims an unsold lot.
pub fn cancel(env: &Env, auction_id: u64) -> Result<(), SaleError> {
    let mut auction = get_auction(env, auction_id).ok_or(SaleError::AuctionNotFound)?;
    if !auction.open {
        return Err(SaleError::AuctionClosed);
    }
    auction.seller.require_auth();
    token::Client::new(env, &auction.sell_token).transfer(
        &env.current_contract_address(),
        &auction.seller,
        &auction.amount,
    );
    auction.open = false;
    set_auction(env, &auction);
    Ok(())
}
