//! Human amount → raw token units.

use ifx_raydium::constants::NATIVE_MINT;
use solana_sdk::pubkey::Pubkey;

pub fn mint_decimals(mint: &Pubkey) -> u8 {
    if *mint == NATIVE_MINT {
        9
    } else {
        6
    }
}

pub fn parse_human_amount(mint: &Pubkey, amount: &str) -> Result<u64, String> {
    let amount = amount.trim();
    if amount.is_empty() {
        return Err("amount is empty".into());
    }
    let decimals = mint_decimals(mint);
    let value: f64 = amount
        .parse()
        .map_err(|_| format!("invalid amount: {amount}"))?;
    if !value.is_finite() || value <= 0.0 {
        return Err("amount must be positive".into());
    }
    let scale = 10f64.powi(decimals as i32);
    let raw = (value * scale).round();
    if raw < 1.0 {
        return Err("amount too small".into());
    }
    Ok(raw as u64)
}

pub fn format_raw_amount(raw: u64, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let scale = 10u64.pow(decimals as u32);
    let whole = raw / scale;
    let frac = raw % scale;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:0width$}", frac, width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{whole}.{trimmed}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sol() {
        let sol = NATIVE_MINT;
        assert_eq!(parse_human_amount(&sol, "0.01").unwrap(), 10_000_000);
    }
}
