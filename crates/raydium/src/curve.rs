//! Constant-product swap curve — mirrors Raydium `CurveCalculator::swap_base_input`.

use crate::fees::{creator_fee, split_creator_fee, trading_fee};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeDirection {
    ZeroForOne,
    OneForZero,
}

pub fn is_creator_fee_on_input(creator_fee_on: u8, direction: TradeDirection) -> bool {
    match (creator_fee_on, direction) {
        (0, _) => true,
        (1, TradeDirection::ZeroForOne) => true,
        (2, TradeDirection::OneForZero) => true,
        _ => false,
    }
}

fn swap_without_fees(
    input_amount: u128,
    input_vault_amount: u128,
    output_vault_amount: u128,
) -> u128 {
    let numerator = input_amount.saturating_mul(output_vault_amount);
    let denominator = input_vault_amount.saturating_add(input_amount);
    numerator / denominator
}

/// On-chain `output_amount` before output-side transfer fees.
pub fn swap_base_input(
    input_amount: u128,
    input_vault_amount: u128,
    output_vault_amount: u128,
    trade_fee_rate: u64,
    creator_fee_rate: u64,
    is_creator_fee_on_input: bool,
) -> Option<(u128, u128)> {
    if input_amount == 0 || input_vault_amount == 0 || output_vault_amount == 0 {
        return None;
    }

    let (trade_fee_paid, input_amount_less_fees) = if is_creator_fee_on_input {
        let total_fee = trading_fee(input_amount, trade_fee_rate + creator_fee_rate)?;
        let creator_part = split_creator_fee(total_fee, trade_fee_rate, creator_fee_rate)?;
        let trade_part = total_fee.saturating_sub(creator_part);
        (trade_part, input_amount.checked_sub(total_fee)?)
    } else {
        let trade_part = trading_fee(input_amount, trade_fee_rate)?;
        (trade_part, input_amount.checked_sub(trade_part)?)
    };

    if input_amount_less_fees == 0 {
        return None;
    }

    let output_swapped = swap_without_fees(
        input_amount_less_fees,
        input_vault_amount,
        output_vault_amount,
    );

    let output_amount = if is_creator_fee_on_input {
        output_swapped
    } else {
        let creator_part = creator_fee(output_swapped, creator_fee_rate)?;
        output_swapped.checked_sub(creator_part)?
    };

    Some((output_amount, trade_fee_paid))
}
