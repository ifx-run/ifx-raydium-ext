//! Dynamic platform fee on proceeds (ported from pumpfun-ext `service-fee.ts`).

use ifx_sdk::expr;
use ifx_sdk::patched_cpi::{
    build_structured_cpi, frame_value, structured_system_transfer, structured_token_transfer,
    system_transfer_template,
};
use ifx_sdk::scratch::FrameScratch;
use ifx_sdk::ScratchError;
use ifx_sdk::typed::ScratchValue;
use ifx_raydium::constants::NATIVE_MINT;
use ifx_raydium::constants::TOKEN_PROGRAM_ID;
use ifx_raydium::swap::user_ata;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use spl_token_interface::instruction::transfer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProceedsLabel {
    Sol,
    Spl,
}

pub struct ProceedsAccount {
    pub label: ProceedsLabel,
    pub user: Pubkey,
    pub user_token_ata: Pubkey,
}

pub struct DynamicFeeResult {
    pub quote_delta: ScratchValue,
    pub fee: Option<ScratchValue>,
    pub net_quote: Option<ScratchValue>,
}

pub fn append_quote_proceeds_baseline(
    scratch: &mut FrameScratch,
    out: &mut Vec<Instruction>,
    account: &ProceedsAccount,
) -> Result<ScratchValue, ScratchError> {
    let mut batch = scratch.let_builder();
    let before = let_quote_proceeds(&mut batch, account)?;
    out.push(batch.build_ix()?);
    Ok(before)
}

pub fn append_proceeds_after_swap(
    scratch: &mut FrameScratch,
    out: &mut Vec<Instruction>,
    account: &ProceedsAccount,
    quote_before: &ScratchValue,
    service_fee_bps: u16,
    fee_recipient: Option<Pubkey>,
    output_mint: Pubkey,
    output_token_program: Pubkey,
) -> Result<DynamicFeeResult, ScratchError> {
    let mut after = scratch.let_builder();
    let quote_after = let_quote_proceeds(&mut after, account)?;
    let quote_delta = after.let_eval(expr::sub(
        expr::r(&quote_after),
        expr::r(quote_before),
    ))?;

    if service_fee_bps == 0 {
        out.push(after.build_ix()?);
        return Ok(DynamicFeeResult {
            quote_delta,
            fee: None,
            net_quote: None,
        });
    }

    let recipient = fee_recipient.ok_or_else(|| {
        ScratchError::BindingType("fee_recipient required when service_fee_bps > 0".into())
    })?;

    let fee = after.let_eval(expr::bps_mul_floor(
        expr::r(&quote_delta),
        expr::u64(service_fee_bps as u64),
    ))?;
    let net_quote = after.let_eval(expr::sub(expr::r(&quote_delta), expr::r(&fee)))?;
    out.push(after.build_ix()?);

    match account.label {
        ProceedsLabel::Sol => {
            out.push(patched_sol_transfer(scratch, account.user, recipient, &fee)?);
        }
        ProceedsLabel::Spl => {
            let dest = user_ata(&recipient, &output_mint, &output_token_program);
            let template = transfer(
                &output_token_program,
                &account.user_token_ata,
                &dest,
                &account.user,
                &[],
                0,
            )
            .map_err(|e| ScratchError::BindingType(e.to_string()))?;
            out.push(patched_token_transfer(scratch, &template, &fee)?);
        }
    }

    Ok(DynamicFeeResult {
        quote_delta,
        fee: Some(fee),
        net_quote: Some(net_quote),
    })
}

pub fn static_service_fee_transfer(
    label: ProceedsLabel,
    fee_raw: u64,
    user: Pubkey,
    user_token_ata: Pubkey,
    recipient: Pubkey,
    quote_mint: Pubkey,
    quote_token_program: Pubkey,
) -> Result<Instruction, ScratchError> {
    if fee_raw == 0 {
        return Err(ScratchError::BindingType("fee must be positive".into()));
    }
    match label {
        ProceedsLabel::Sol => Ok(solana_system_interface::instruction::transfer(
            &user,
            &recipient,
            fee_raw,
        )),
        ProceedsLabel::Spl => {
            let dest = get_associated_token_address_with_program_id(
                &recipient,
                &quote_mint,
                &quote_token_program,
            );
            transfer(
                &quote_token_program,
                &user_token_ata,
                &dest,
                &user,
                &[],
                fee_raw,
            )
            .map_err(|e| ScratchError::BindingType(e.to_string()))
        }
    }
}

fn let_quote_proceeds(
    batch: &mut ifx_sdk::LetBuilder<'_>,
    account: &ProceedsAccount,
) -> Result<ScratchValue, ScratchError> {
    match account.label {
        ProceedsLabel::Sol => batch.lamports(account.user),
        ProceedsLabel::Spl => batch.spl_token_amount(account.user_token_ata),
    }
}

fn patched_sol_transfer(
    scratch: &mut FrameScratch,
    from: Pubkey,
    to: Pubkey,
    amount: &ScratchValue,
) -> Result<Instruction, ScratchError> {
    let template = system_transfer_template(from, to);
    let built = build_structured_cpi(&template, structured_system_transfer(frame_value(amount)))?;
    scratch.ix_cpi(&built)
}

fn patched_token_transfer(
    scratch: &mut FrameScratch,
    template: &Instruction,
    amount: &ScratchValue,
) -> Result<Instruction, ScratchError> {
    let built = build_structured_cpi(
        template,
        structured_token_transfer(frame_value(amount)),
    )?;
    scratch.ix_cpi(&built)
}

pub fn proceeds_label_for_mint(mint: &Pubkey) -> ProceedsLabel {
    if *mint == NATIVE_MINT {
        ProceedsLabel::Sol
    } else {
        ProceedsLabel::Spl
    }
}

/// Raydium CPMM settles the SOL side via WSOL ATA — measure proceeds on SPL balance, not wallet lamports.
pub fn wsol_proceeds_account(user: Pubkey) -> ProceedsAccount {
    ProceedsAccount {
        label: ProceedsLabel::Spl,
        user,
        user_token_ata: user_ata(&user, &NATIVE_MINT, &TOKEN_PROGRAM_ID),
    }
}

pub fn idempotent_ata_create(
    payer: Pubkey,
    owner: Pubkey,
    mint: Pubkey,
    token_program: Pubkey,
) -> Instruction {
    spl_associated_token_account::instruction::create_associated_token_account_idempotent(
        &payer,
        &owner,
        &mint,
        &token_program,
    )
}
