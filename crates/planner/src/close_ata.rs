//! Conditionally close an empty SPL token ATA when balance is zero (Ifx if_else).

use ifx_sdk::expr;
use ifx_sdk::if_else::{args, cpi, skip};
use ifx_sdk::patched_cpi::{build_static_cpi, with_owner_signer};
use ifx_sdk::scratch::FrameScratch;
use ifx_sdk::ScratchError;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use spl_token_interface::instruction as spl_ix;

const TOKEN_2022_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

#[derive(Debug, Clone)]
pub struct CloseAtaCandidate {
    pub token_account: Pubkey,
    pub rent_destination: Pubkey,
    pub owner: Pubkey,
    pub token_program: Pubkey,
}

pub fn append_conditional_close_ata(
    scratch: &mut FrameScratch,
    out: &mut Vec<Instruction>,
    params: &CloseAtaCandidate,
) -> Result<(), ScratchError> {
    let CloseAtaCandidate {
        token_account,
        rent_destination,
        owner,
        token_program,
    } = *params;

    let mut batch = scratch.let_builder();
    let balance = if token_program == TOKEN_2022_PROGRAM_ID {
        batch.spl_token2022_amount(token_account)?
    } else {
        batch.spl_token_amount(token_account)?
    };
    out.push(batch.build_ix()?);

    let close_tpl = spl_ix::close_account(
        &token_program,
        &token_account,
        &rent_destination,
        &owner,
        &[],
    )
    .map_err(|e| ScratchError::BindingType(e.to_string()))?;
    let close_ix = with_owner_signer(&close_tpl, owner, true);
    let close_built = build_static_cpi(&close_ix)?;

    out.push(scratch.ix_if_else(
        &args(
            expr::is_zero(expr::r(&balance)),
            cpi(close_built.cpi.clone()),
            skip(),
        ),
        &close_built.remaining,
    )?);

    Ok(())
}

pub fn build_smart_close_instructions(
    scratch: &mut FrameScratch,
    candidates: &[CloseAtaCandidate],
) -> Result<Vec<Instruction>, ScratchError> {
    let mut ixs = Vec::new();
    for candidate in candidates {
        append_conditional_close_ata(scratch, &mut ixs, candidate)?;
    }
    Ok(ixs)
}
