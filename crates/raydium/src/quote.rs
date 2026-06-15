//! Local CPMM constant-product quote aligned with Raydium on-chain math.

use crate::curve::{is_creator_fee_on_input, swap_base_input, TradeDirection};
use crate::pool::{CpmmPool, PoolSide};
use crate::rpc::effective_vault_amounts;

#[derive(Debug, Clone)]
pub struct SwapQuote {
    /// Net amount passed into the Raydium swap (after platform fee when input is WSOL).
    pub amount_in: u64,
    /// User-facing pay total; equals `amount_in` when no input-side platform fee.
    pub gross_amount_in: u64,
    /// Platform fee taken from `gross_amount_in` when input is WSOL.
    pub platform_fee: u64,
    pub expected_out: u64,
    pub min_out: u64,
    pub trade_fee_paid: u64,
    pub input_mint: solana_sdk::pubkey::Pubkey,
    pub output_mint: solana_sdk::pubkey::Pubkey,
}

pub fn quote_swap_base_input(
    pool: &CpmmPool,
    input_mint: &solana_sdk::pubkey::Pubkey,
    amount_in: u64,
    slippage_bps: u16,
) -> Option<SwapQuote> {
    let side = pool.side_of(input_mint)?;
    let output_mint = pool.other_mint(input_mint)?;
    let direction = match side {
        PoolSide::A => TradeDirection::ZeroForOne,
        PoolSide::B => TradeDirection::OneForZero,
    };

    let (reserve_0, reserve_1) = effective_vault_amounts(pool);
    let (reserve_in, reserve_out) = match side {
        PoolSide::A => (reserve_0, reserve_1),
        PoolSide::B => (reserve_1, reserve_0),
    };

    let creator_fee_rate = if pool.enable_creator_fee {
        pool.creator_fee_rate
    } else {
        0
    };
    let fee_on_input = is_creator_fee_on_input(pool.creator_fee_on, direction);

    let (expected_out, trade_fee_paid) = swap_base_input(
        u128::from(amount_in),
        u128::from(reserve_in),
        u128::from(reserve_out),
        pool.trade_fee_rate,
        creator_fee_rate,
        fee_on_input,
    )?;

    let expected_out = u64::try_from(expected_out).ok()?;
    let trade_fee_paid = u64::try_from(trade_fee_paid).ok()?;
    let min_out = min_amount_out(expected_out, slippage_bps);

    Some(SwapQuote {
        amount_in,
        gross_amount_in: amount_in,
        platform_fee: 0,
        expected_out,
        min_out,
        trade_fee_paid,
        input_mint: *input_mint,
        output_mint: output_mint.address,
    })
}

pub fn min_amount_out(expected_out: u64, slippage_bps: u16) -> u64 {
    let num = (expected_out as u128).saturating_mul(10_000u128 - slippage_bps as u128);
    (num / 10_000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::MintInfo;
    use solana_sdk::pubkey::Pubkey;

    fn sample_pool() -> CpmmPool {
        CpmmPool {
            pool_id: Pubkey::new_unique(),
            amm_config: Pubkey::new_unique(),
            authority: Pubkey::new_unique(),
            observation_state: Pubkey::new_unique(),
            token_0_mint: MintInfo {
                address: Pubkey::new_unique(),
                token_program: Pubkey::new_unique(),
                decimals: 9,
                symbol: "A".into(),
            },
            token_1_mint: MintInfo {
                address: Pubkey::new_unique(),
                token_program: Pubkey::new_unique(),
                decimals: 6,
                symbol: "B".into(),
            },
            token_0_vault: Pubkey::new_unique(),
            token_1_vault: Pubkey::new_unique(),
            token_0_program: Pubkey::new_unique(),
            token_1_program: Pubkey::new_unique(),
            trade_fee_rate: 2_500,
            creator_fee_rate: 0,
            enable_creator_fee: false,
            creator_fee_on: 0,
            protocol_fees_token_0: 0,
            protocol_fees_token_1: 0,
            fund_fees_token_0: 0,
            fund_fees_token_1: 0,
            creator_fees_token_0: 0,
            creator_fees_token_1: 0,
            vault_0_amount: 1_000_000_000,
            vault_1_amount: 100_000_000,
        }
    }

    #[test]
    fn quote_returns_positive_out() {
        let pool = sample_pool();
        let q = quote_swap_base_input(&pool, &pool.token_0_mint.address, 10_000_000, 100).unwrap();
        assert!(q.expected_out > 0);
        assert!(q.min_out <= q.expected_out);
    }

    #[test]
    fn accrued_fees_on_output_side_reduce_quote() {
        let mut pool = sample_pool();
        pool.fund_fees_token_1 = 2_000_000;
        let with_accrued = quote_swap_base_input(&pool, &pool.token_0_mint.address, 10_000_000, 0).unwrap();
        pool.fund_fees_token_1 = 0;
        let without_accrued =
            quote_swap_base_input(&pool, &pool.token_0_mint.address, 10_000_000, 0).unwrap();
        assert!(without_accrued.expected_out > with_accrued.expected_out);
    }

    #[test]
    fn trading_fee_uses_ceil_not_floor() {
        let mut pool = sample_pool();
        pool.trade_fee_rate = 25; // 0.0025%
        let q = quote_swap_base_input(&pool, &pool.token_0_mint.address, 1_000, 0).unwrap();
        // ceil(1000 * 25 / 1_000_000) = 1, floor would be 0
        assert_eq!(q.trade_fee_paid, 1);
    }
}
