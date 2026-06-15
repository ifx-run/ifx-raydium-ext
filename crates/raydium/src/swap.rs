//! Raydium CPMM `swap_base_input` instruction builder.

use crate::constants::{
    CPMM_PROGRAM_ID, SWAP_BASE_INPUT_AMOUNT_IN_OFFSET, SWAP_BASE_INPUT_DISCRIMINATOR,
    SWAP_BASE_INPUT_MIN_OUT_OFFSET,
};
use crate::pool::{CpmmPool, PoolSide};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use spl_associated_token_account::get_associated_token_address_with_program_id;

#[derive(Debug, Clone, Copy)]
pub enum SwapDirection {
    /// Exact input on token_0 → token_1.
    ZeroToOne,
    /// Exact input on token_1 → token_0.
    OneToZero,
}

#[derive(Debug, Clone)]
pub struct SwapBuildParams {
    pub pool: CpmmPool,
    pub user: Pubkey,
    pub direction: SwapDirection,
    pub amount_in: u64,
    pub minimum_amount_out: u64,
}

pub fn user_ata(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    get_associated_token_address_with_program_id(owner, mint, token_program)
}

pub fn wrap_sol_ixs(user: &Pubkey, wsol_ata: Pubkey, lamports: u64) -> Vec<Instruction> {
    if lamports == 0 {
        return vec![];
    }
    vec![
        solana_system_interface::instruction::transfer(user, &wsol_ata, lamports),
        spl_token_interface::instruction::sync_native(&crate::constants::TOKEN_PROGRAM_ID, &wsol_ata)
            .expect("sync_native instruction"),
    ]
}

/// Unwrap WSOL ATA → native SOL in the user's wallet (Raydium swap out credits WSOL).
pub fn close_wsol_ix(user: &Pubkey, wsol_ata: Pubkey) -> Instruction {
    spl_token_interface::instruction::close_account(
        &crate::constants::TOKEN_PROGRAM_ID,
        &wsol_ata,
        user,
        user,
        &[],
    )
    .expect("close_account instruction")
}

pub fn swap_base_input_ix(params: &SwapBuildParams) -> Instruction {
    let SwapBuildParams {
        pool,
        user,
        direction,
        amount_in,
        minimum_amount_out,
    } = params;

    let (input_mint, output_mint, input_program, output_program, input_vault, output_vault) =
        match direction {
            SwapDirection::ZeroToOne => (
                &pool.token_0_mint.address,
                &pool.token_1_mint.address,
                &pool.token_0_program,
                &pool.token_1_program,
                pool.token_0_vault,
                pool.token_1_vault,
            ),
            SwapDirection::OneToZero => (
                &pool.token_1_mint.address,
                &pool.token_0_mint.address,
                &pool.token_1_program,
                &pool.token_0_program,
                pool.token_1_vault,
                pool.token_0_vault,
            ),
        };

    let input_ata = user_ata(user, input_mint, input_program);
    let output_ata = user_ata(user, output_mint, output_program);

    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&SWAP_BASE_INPUT_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&minimum_amount_out.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(*user, true),
        AccountMeta::new_readonly(pool.authority, false),
        AccountMeta::new_readonly(pool.amm_config, false),
        AccountMeta::new(pool.pool_id, false),
        AccountMeta::new(input_ata, false),
        AccountMeta::new(output_ata, false),
        AccountMeta::new(input_vault, false),
        AccountMeta::new(output_vault, false),
        AccountMeta::new_readonly(*input_program, false),
        AccountMeta::new_readonly(*output_program, false),
        AccountMeta::new_readonly(*input_mint, false),
        AccountMeta::new_readonly(*output_mint, false),
        AccountMeta::new(pool.observation_state, false),
    ];

    Instruction {
        program_id: CPMM_PROGRAM_ID,
        accounts,
        data,
    }
}

pub fn swap_base_input_template(params: &SwapBuildParams) -> Instruction {
    let mut p = params.clone();
    p.amount_in = 0;
    swap_base_input_ix(&p)
}

pub fn direction_for_input(pool: &CpmmPool, input_mint: &Pubkey) -> Option<SwapDirection> {
    match pool.side_of(input_mint)? {
        PoolSide::A => Some(SwapDirection::ZeroToOne),
        PoolSide::B => Some(SwapDirection::OneToZero),
    }
}

pub const AMOUNT_IN_PATCH_OFFSET: u16 = SWAP_BASE_INPUT_AMOUNT_IN_OFFSET;
pub const MIN_OUT_PATCH_OFFSET: u16 = SWAP_BASE_INPUT_MIN_OUT_OFFSET;
