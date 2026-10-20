//! Leveraged margin positions with mark-price liquidation.
//!
//! A position locks `margin = ceil(size / leverage)` of collateral. Its liquidation price sits where
//! the remaining margin falls to the maintenance ratio:
//!
//! - long:  `entry * (1 - 1/leverage + maintenance)`
//! - short: `entry * (1 + 1/leverage - maintenance)`
//!
//! Anyone may `liquidate` once the mark price crosses it and earns a 5% bounty on the margin.
//! The realised loss is capped at the margin, so a position can never owe more than it locked.

use soroban_sdk::{contracterror, contracttype, token, Address, Env};

pub const MAX_LEVERAGE: u32 = 10;
/// Maintenance margin ratio, basis points of notional.
pub const MAINTENANCE_BPS: i128 = 500;
/// Liquidator bounty, basis points of the position margin.
pub const LIQUIDATION_BOUNTY_BPS: i128 = 500;
const BPS: i128 = 10_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MarginError {
    NotInitialized = 101,
    MarkPriceNotSet = 102,
    InvalidSize = 103,
    InvalidLeverage = 104,
    PositionNotFound = 105,
    PositionClosed = 106,
    NotLiquidatable = 107,
    Overflow = 108,
    InvalidPrice = 109,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MarginPosition {
    pub id: u64,
    pub user: Address,
    pub market_id: u64,
    pub token: Address,
    pub size: i128,
    pub margin: i128,
    pub leverage: u32,
    pub is_long: bool,
    pub entry_price: i128,
    pub liquidation_price: i128,
    pub open: bool,
}

#[contracttype]
pub enum MarginKey {
    MarkPrice(u64),
    NextPositionId,
    Position(u64),
}

pub fn get_mark_price(env: &Env, market_id: u64) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&MarginKey::MarkPrice(market_id))
}

pub fn set_mark_price(env: &Env, market_id: u64, price: i128) {
    env.storage()
        .persistent()
        .set(&MarginKey::MarkPrice(market_id), &price);
}

pub fn get_position(env: &Env, id: u64) -> Option<MarginPosition> {
    env.storage().persistent().get(&MarginKey::Position(id))
}

fn set_position(env: &Env, position: &MarginPosition) {
    env.storage()
        .persistent()
        .set(&MarginKey::Position(position.id), position);
}

pub fn next_position_id(env: &Env) -> u64 {
    let next: u64 = env
        .storage()
        .persistent()
        .get(&MarginKey::NextPositionId)
        .unwrap_or(0)
        + 1;
    env.storage()
        .persistent()
        .set(&MarginKey::NextPositionId, &next);
    next
}

/// Required margin for `size` at `leverage` (rounded up so the position is never under-collateralised).
pub fn required_margin(size: i128, leverage: u32) -> Result<i128, MarginError> {
    if size <= 0 {
        return Err(MarginError::InvalidSize);
    }
    if leverage == 0 || leverage > MAX_LEVERAGE {
        return Err(MarginError::InvalidLeverage);
    }
    let lev = i128::from(leverage);
    size.checked_add(lev - 1)
        .ok_or(MarginError::Overflow)
        .map(|v| v / lev)
}

/// Mark price at which a position opened at `entry_price` becomes liquidatable.
pub fn liquidation_price(
    entry_price: i128,
    leverage: u32,
    is_long: bool,
) -> Result<i128, MarginError> {
    let buffer = BPS / i128::from(leverage);
    let factor = if is_long {
        BPS - buffer + MAINTENANCE_BPS
    } else {
        BPS + buffer - MAINTENANCE_BPS
    };
    entry_price
        .checked_mul(factor)
        .map(|v| v / BPS)
        .ok_or(MarginError::Overflow)
}

pub fn is_liquidatable(position: &MarginPosition, mark_price: i128) -> bool {
    if position.is_long {
        mark_price <= position.liquidation_price
    } else {
        mark_price >= position.liquidation_price
    }
}

pub fn open(
    env: &Env,
    user: &Address,
    market_id: u64,
    token_address: &Address,
    size: i128,
    leverage: u32,
    is_long: bool,
) -> Result<u64, MarginError> {
    let margin = required_margin(size, leverage)?;
    let entry_price = get_mark_price(env, market_id).ok_or(MarginError::MarkPriceNotSet)?;
    let liquidation_price = liquidation_price(entry_price, leverage, is_long)?;

    token::Client::new(env, token_address).transfer(user, &env.current_contract_address(), &margin);

    let position = MarginPosition {
        id: next_position_id(env),
        user: user.clone(),
        market_id,
        token: token_address.clone(),
        size,
        margin,
        leverage,
        is_long,
        entry_price,
        liquidation_price,
        open: true,
    };
    set_position(env, &position);
    Ok(position.id)
}

/// Settles a liquidation: loss (capped at margin) stays in the contract, the liquidator gets the
/// bounty and the owner gets whatever margin is left.
pub fn liquidate(env: &Env, liquidator: &Address, position_id: u64) -> Result<i128, MarginError> {
    let mut position = get_position(env, position_id).ok_or(MarginError::PositionNotFound)?;
    if !position.open {
        return Err(MarginError::PositionClosed);
    }
    let mark = get_mark_price(env, position.market_id).ok_or(MarginError::MarkPriceNotSet)?;
    if !is_liquidatable(&position, mark) {
        return Err(MarginError::NotLiquidatable);
    }

    let move_against = if position.is_long {
        position.entry_price - mark
    } else {
        mark - position.entry_price
    };
    let loss = position
        .size
        .checked_mul(move_against.max(0))
        .ok_or(MarginError::Overflow)?
        / position.entry_price;
    let loss = loss.min(position.margin);
    let remaining = position.margin - loss;
    let bounty = (position.margin * LIQUIDATION_BOUNTY_BPS / BPS).min(remaining);

    let client = token::Client::new(env, &position.token);
    let contract = env.current_contract_address();
    if bounty > 0 {
        client.transfer(&contract, liquidator, &bounty);
    }
    if remaining - bounty > 0 {
        client.transfer(&contract, &position.user, &(remaining - bounty));
    }

    position.open = false;
    set_position(env, &position);
    Ok(bounty)
}
