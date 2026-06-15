//! How the user funds a swap when mint A is WSOL (`NATIVE_MINT`).

use ifx_raydium::constants::NATIVE_MINT;
use solana_sdk::pubkey::Pubkey;

/// Raydium pool mint `So11…` is WSOL. Users may pay from WSOL ATA or native SOL (wrap in-tx).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SolPayAsset {
    /// Spend SPL balance in the user's WSOL ATA (no wrap).
    Wsol,
    /// Spend wallet lamports; transaction wraps into WSOL ATA before swap.
    #[default]
    NativeSol,
}

impl SolPayAsset {
    pub fn as_str(self) -> &'static str {
        match self {
            SolPayAsset::Wsol => "wsol",
            SolPayAsset::NativeSol => "native_sol",
        }
    }
}

pub fn sol_pay_asset_for_mint(mint: &Pubkey, raw: Option<&str>) -> Option<SolPayAsset> {
    if *mint != NATIVE_MINT {
        return None;
    }
    let s = raw.unwrap_or("native_sol").trim().to_ascii_lowercase();
    Some(match s.as_str() {
        "native_sol" | "native" | "sol" => SolPayAsset::NativeSol,
        _ => SolPayAsset::Wsol,
    })
}
