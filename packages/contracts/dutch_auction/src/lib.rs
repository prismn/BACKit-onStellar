#![no_std]

mod errors;
mod events;
pub mod sale;
mod storage;
mod types;

pub use types::AuctionInfo;

#[cfg(test)]
mod test;

use errors::DutchAuctionError;
use events::{emit_auth_params_changed, emit_dutch_auction_settled, emit_dutch_auction_started};
use sale::{SaleAuction, SaleError};
use soroban_sdk::{contract, contractimpl, Address, Env};
use storage::{get_config, set_auction_info, set_config};
use types::DutchAuctionConfig;

fn require_admin(env: &Env) -> Result<Address, DutchAuctionError> {
    let config = get_config(env).ok_or(DutchAuctionError::NotInitialized)?;
    config.admin.require_auth();
    Ok(config.admin)
}

#[contract]
pub struct DutchAuction;

#[contractimpl]
impl DutchAuction {
    pub fn initialize(
        env: Env,
        admin: Address,
        outcome_manager: Address,
        auction_duration_secs: u64,
        oracle_deadline_secs: u64,
        settler_reward_bps: u32,
    ) -> Result<(), DutchAuctionError> {
        if get_config(&env).is_some() {
            return Err(DutchAuctionError::AlreadyInitialized);
        }
        if settler_reward_bps > 10_000 {
            return Err(DutchAuctionError::InvalidParams);
        }
        if auction_duration_secs == 0 {
            return Err(DutchAuctionError::InvalidParams);
        }
        let config = DutchAuctionConfig {
            admin,
            outcome_manager,
            auction_duration_secs,
            oracle_deadline_secs,
            settler_reward_bps,
        };
        set_config(&env, &config);
        Ok(())
    }

    pub fn start_dutch_auction(
        env: Env,
        call_id: u64,
        start_price: i128,
        condition_type: u32,
    ) -> Result<(), DutchAuctionError> {
        let config = get_config(&env).ok_or(DutchAuctionError::NotInitialized)?;
        if condition_type != 1 && condition_type != 2 {
            return Err(DutchAuctionError::UnknownConditionType);
        }
        if start_price <= 0 {
            return Err(DutchAuctionError::InvalidPrice);
        }
        if storage::get_auction_info(&env, call_id).is_some() {
            let existing = storage::get_auction_info(&env, call_id).unwrap();
            if existing.settled {
                return Err(DutchAuctionError::AuctionAlreadySettled);
            }
            return Err(DutchAuctionError::AuctionAlreadySettled);
        }
        let now = env.ledger().timestamp();
        let info = AuctionInfo {
            call_id,
            condition_type,
            start_price,
            start_ts: now,
            settled: false,
        };
        set_auction_info(&env, call_id, &info);
        emit_dutch_auction_started(&env, call_id, start_price, condition_type, now);
        Ok(())
    }

    pub fn settle_dutch_auction(
        env: Env,
        caller: Address,
        call_id: u64,
    ) -> Result<i128, DutchAuctionError> {
        caller.require_auth();
        let config = get_config(&env).ok_or(DutchAuctionError::NotInitialized)?;
        let mut info =
            storage::get_auction_info(&env, call_id).ok_or(DutchAuctionError::AuctionNotStarted)?;
        if info.settled {
            return Err(DutchAuctionError::AuctionAlreadySettled);
        }
        let price = Self::get_dutch_auction_price(env.clone(), call_id)?;
        let reward = price * config.settler_reward_bps as i128 / 10_000;
        info.settled = true;
        set_auction_info(&env, call_id, &info);
        emit_dutch_auction_settled(&env, call_id, price, &caller, reward);
        Ok(price)
    }

    pub fn get_dutch_auction_price(env: Env, call_id: u64) -> Result<i128, DutchAuctionError> {
        let config = get_config(&env).ok_or(DutchAuctionError::NotInitialized)?;
        let info =
            storage::get_auction_info(&env, call_id).ok_or(DutchAuctionError::AuctionNotStarted)?;
        let now = env.ledger().timestamp();
        let elapsed = now.saturating_sub(info.start_ts);
        let duration = config.auction_duration_secs;
        if elapsed >= duration {
            return match info.condition_type {
                1 => Ok(0),
                2 => Ok(info.start_price * 2),
                _ => Err(DutchAuctionError::UnknownConditionType),
            };
        }
        match info.condition_type {
            1 => Ok(info.start_price * 2 * (duration as i128 - elapsed as i128) / duration as i128),
            2 => {
                let numerator = info.start_price * (duration as i128 + 3 * elapsed as i128);
                Ok(numerator / (2 * duration as i128))
            }
            _ => Err(DutchAuctionError::UnknownConditionType),
        }
    }

    pub fn get_auction_info(env: Env, call_id: u64) -> Option<AuctionInfo> {
        storage::get_auction_info(&env, call_id)
    }

    pub fn update_params(
        env: Env,
        auction_duration_secs: u64,
        oracle_deadline_secs: u64,
        settler_reward_bps: u32,
    ) -> Result<(), DutchAuctionError> {
        require_admin(&env)?;
        if settler_reward_bps > 10_000 {
            return Err(DutchAuctionError::InvalidParams);
        }
        if auction_duration_secs == 0 {
            return Err(DutchAuctionError::InvalidParams);
        }
        let mut config = get_config(&env).ok_or(DutchAuctionError::NotInitialized)?;
        config.auction_duration_secs = auction_duration_secs;
        config.oracle_deadline_secs = oracle_deadline_secs;
        config.settler_reward_bps = settler_reward_bps;
        set_config(&env, &config);
        emit_auth_params_changed(
            &env,
            auction_duration_secs,
            oracle_deadline_secs,
            settler_reward_bps,
        );
        Ok(())
    }
}

#[contractimpl]
impl DutchAuction {
    /// Escrow `amount` of `sell_token` and start a falling-price sale paid in `pay_token`.
    /// `curve`: 0 = linear decay, 1 = quadratic decay. Returns the auction id.
    #[allow(clippy::too_many_arguments)]
    pub fn create_auction(
        env: Env,
        seller: Address,
        sell_token: Address,
        pay_token: Address,
        amount: i128,
        start_price: i128,
        floor_price: i128,
        duration: u64,
        curve: u32,
    ) -> Result<u64, SaleError> {
        seller.require_auth();
        sale::create(
            &env,
            &seller,
            &sell_token,
            &pay_token,
            amount,
            start_price,
            floor_price,
            duration,
            curve,
        )
    }

    /// Buy the lot at the exact current price; fails if it exceeds `max_price`. Emits `AuctionFilled`.
    pub fn buy(
        env: Env,
        buyer: Address,
        auction_id: u64,
        max_price: i128,
    ) -> Result<i128, SaleError> {
        buyer.require_auth();
        let price = sale::buy(&env, &buyer, auction_id, max_price)?;
        events::emit_auction_filled(&env, auction_id, &buyer, price);
        Ok(price)
    }

    /// Seller cancels an unsold auction and reclaims the lot.
    pub fn cancel_auction(env: Env, auction_id: u64) -> Result<(), SaleError> {
        sale::cancel(&env, auction_id)
    }

    /// Current price of a sale auction.
    pub fn get_sale_price(env: Env, auction_id: u64) -> Result<i128, SaleError> {
        let auction = sale::get_auction(&env, auction_id).ok_or(SaleError::AuctionNotFound)?;
        sale::price_at(&auction, env.ledger().timestamp())
    }

    pub fn get_sale_auction(env: Env, auction_id: u64) -> Result<SaleAuction, SaleError> {
        sale::get_auction(&env, auction_id).ok_or(SaleError::AuctionNotFound)
    }
}
