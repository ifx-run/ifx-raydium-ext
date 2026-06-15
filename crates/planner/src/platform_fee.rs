//! Input-side platform fee when mint A is WSOL (`NATIVE_MINT`).

use ifx_raydium::constants::NATIVE_MINT;
use ifx_raydium::pool::CpmmPool;
use ifx_raydium::quote::{quote_swap_base_input, SwapQuote};
use solana_sdk::pubkey::Pubkey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputFeeSplit {
    pub gross: u64,
    pub platform_fee: u64,
    pub swap_in: u64,
}

pub fn split_gross_input(
    input_mint: &Pubkey,
    gross: u64,
    service_fee_bps: u16,
) -> Option<InputFeeSplit> {
    if gross == 0 {
        return None;
    }
    if *input_mint != NATIVE_MINT || service_fee_bps == 0 {
        return Some(InputFeeSplit {
            gross,
            platform_fee: 0,
            swap_in: gross,
        });
    }
    let platform_fee = (gross as u128 * service_fee_bps as u128 / 10_000) as u64;
    if platform_fee >= gross {
        return None;
    }
    Some(InputFeeSplit {
        gross,
        platform_fee,
        swap_in: gross - platform_fee,
    })
}

pub fn net_proceeds_after_fee(gross: u64, service_fee_bps: u16) -> u64 {
    split_gross_input(&NATIVE_MINT, gross, service_fee_bps)
        .map(|s| s.swap_in)
        .unwrap_or(gross)
}

pub fn quote_swap_with_platform_fee(
    pool: &CpmmPool,
    input_mint: &Pubkey,
    gross_amount_in: u64,
    slippage_bps: u16,
    service_fee_bps: u16,
) -> Option<SwapQuote> {
    let split = split_gross_input(input_mint, gross_amount_in, service_fee_bps)?;
    let mut quote = quote_swap_base_input(pool, input_mint, split.swap_in, slippage_bps)?;
    quote.gross_amount_in = split.gross;
    quote.platform_fee = split.platform_fee;
    Some(quote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ifx_raydium::constants::NATIVE_MINT;
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn splits_platform_fee_from_gross_wsol_input() {
        let gross = 10_000_000u64; // 0.01 SOL
        let split = split_gross_input(&NATIVE_MINT, gross, 5).unwrap();
        assert_eq!(split.gross, gross);
        assert_eq!(split.platform_fee, 5_000);
        assert_eq!(split.swap_in, 9_995_000);
    }

    #[test]
    fn leaves_spl_input_unsplit() {
        let mint = Pubkey::new_unique();
        let split = split_gross_input(&mint, 1_000_000, 5).unwrap();
        assert_eq!(split.platform_fee, 0);
        assert_eq!(split.swap_in, 1_000_000);
    }
}
