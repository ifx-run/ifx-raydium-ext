//! Raydium CPMM fee math — mirrors `programs/cp-swap/src/curve/fees.rs`.

use crate::constants::FEE_RATE_DENOMINATOR;

fn ceil_div(token_amount: u128, fee_numerator: u128, fee_denominator: u128) -> Option<u128> {
    if fee_denominator == 0 {
        return None;
    }
    token_amount
        .checked_mul(fee_numerator)?
        .checked_add(fee_denominator)?
        .checked_sub(1)?
        .checked_div(fee_denominator)
}

fn floor_div(token_amount: u128, fee_numerator: u128, fee_denominator: u128) -> Option<u128> {
    if fee_denominator == 0 {
        return None;
    }
    token_amount
        .checked_mul(fee_numerator)?
        .checked_div(fee_denominator)
}

pub fn trading_fee(amount: u128, trade_fee_rate: u64) -> Option<u128> {
    ceil_div(
        amount,
        u128::from(trade_fee_rate),
        u128::from(FEE_RATE_DENOMINATOR),
    )
}

pub fn creator_fee(amount: u128, creator_fee_rate: u64) -> Option<u128> {
    if creator_fee_rate == 0 {
        return Some(0);
    }
    ceil_div(
        amount,
        u128::from(creator_fee_rate),
        u128::from(FEE_RATE_DENOMINATOR),
    )
}

pub fn split_creator_fee(total_fee: u128, trade_fee_rate: u64, creator_fee_rate: u64) -> Option<u128> {
    if creator_fee_rate == 0 {
        return Some(0);
    }
    floor_div(
        total_fee,
        u128::from(creator_fee_rate),
        u128::from(trade_fee_rate + creator_fee_rate),
    )
}
