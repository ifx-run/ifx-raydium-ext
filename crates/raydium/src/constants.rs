use solana_sdk::pubkey::Pubkey;

pub const CPMM_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C");

pub const AMM_V4_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");

pub const NATIVE_MINT: Pubkey =
    solana_sdk::pubkey!("So11111111111111111111111111111111111111112");

pub const TOKEN_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

pub const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("ATokenGPvbdGDxr719oRQUvk4RipKdbM5U5YXKjJtW8");

pub const AUTH_SEED: &[u8] = b"vault_and_lp_mint_auth_seed";

/// Raydium CPMM trade fee denominator (trade_fee_rate is in hundredths of a bip).
pub const FEE_RATE_DENOMINATOR: u64 = 1_000_000;

/// Anchor `swap_base_input` discriminator.
pub const SWAP_BASE_INPUT_DISCRIMINATOR: [u8; 8] =
    [0x8f, 0xbe, 0x5a, 0xda, 0xc4, 0x1e, 0x33, 0xde];

/// Byte offset of `amount_in` in `swap_base_input` instruction data.
pub const SWAP_BASE_INPUT_AMOUNT_IN_OFFSET: u16 = 8;

/// Byte offset of `minimum_amount_out` in `swap_base_input` instruction data.
pub const SWAP_BASE_INPUT_MIN_OUT_OFFSET: u16 = 16;

pub const API_V3_BASE: &str = "https://api-v3.raydium.io";
