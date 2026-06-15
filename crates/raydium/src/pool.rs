//! Pool types shared between API hints and on-chain state.

use solana_sdk::pubkey::Pubkey;

#[derive(Debug, Clone)]
pub struct MintInfo {
    pub address: Pubkey,
    pub token_program: Pubkey,
    pub decimals: u8,
    pub symbol: String,
}

/// API-level pool reference (may lack full on-chain keys until hydrated).
#[derive(Debug, Clone)]
pub struct PoolRef {
    pub pool_id: Pubkey,
    pub amm_config: Option<Pubkey>,
    pub mint_a: MintInfo,
    pub mint_b: MintInfo,
    pub trade_fee_rate: Option<u64>,
    pub tvl: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolSide {
    A,
    B,
}

/// Fully hydrated CPMM pool ready for quote / swap ix build.
#[derive(Debug, Clone)]
pub struct CpmmPool {
    pub pool_id: Pubkey,
    pub amm_config: Pubkey,
    pub authority: Pubkey,
    pub observation_state: Pubkey,
    pub token_0_mint: MintInfo,
    pub token_1_mint: MintInfo,
    pub token_0_vault: Pubkey,
    pub token_1_vault: Pubkey,
    pub token_0_program: Pubkey,
    pub token_1_program: Pubkey,
    pub trade_fee_rate: u64,
    pub creator_fee_rate: u64,
    pub enable_creator_fee: bool,
    /// 0 = both tokens, 1 = only token0, 2 = only token1
    pub creator_fee_on: u8,
    pub protocol_fees_token_0: u64,
    pub protocol_fees_token_1: u64,
    pub fund_fees_token_0: u64,
    pub fund_fees_token_1: u64,
    pub creator_fees_token_0: u64,
    pub creator_fees_token_1: u64,
    pub vault_0_amount: u64,
    pub vault_1_amount: u64,
}

impl CpmmPool {
    pub fn mint_0(&self) -> &MintInfo {
        &self.token_0_mint
    }

    pub fn mint_1(&self) -> &MintInfo {
        &self.token_1_mint
    }

    pub fn side_of(&self, mint: &Pubkey) -> Option<PoolSide> {
        if self.token_0_mint.address == *mint {
            Some(PoolSide::A)
        } else if self.token_1_mint.address == *mint {
            Some(PoolSide::B)
        } else {
            None
        }
    }

    pub fn other_mint(&self, input: &Pubkey) -> Option<&MintInfo> {
        match self.side_of(input)? {
            PoolSide::A => Some(&self.token_1_mint),
            PoolSide::B => Some(&self.token_0_mint),
        }
    }
}
