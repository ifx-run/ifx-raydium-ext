//! Sponsor repay — ATA bootstrap, fee math, WSOL settle.

mod fees;

pub use fees::{
    apply_repay_buffer, compute_tx_fee_lamports, priority_fee_lamports_for_tier,
    sponsor_tx_signature_count, LAMPORTS_PER_SIGNATURE,
};

use crate::service_fee::idempotent_ata_create;
use ifx_sdk::expr;
use ifx_sdk::patched_cpi::{
    build_structured_cpi, frame_value, structured_system_transfer, system_transfer_template,
};
use ifx_sdk::scratch::FrameScratch;
use ifx_sdk::ScratchError;
use ifx_sdk::typed::ScratchValue;
use ifx_raydium::swap::user_ata;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;

#[derive(Debug, Clone)]
pub struct AtaSpec {
    pub owner: Pubkey,
    pub mint: Pubkey,
    pub token_program: Pubkey,
}

pub struct SponsorRepayParams {
    pub tx_fee_lamports: u64,
    pub repay_buffer_percent: u16,
    pub ata_cost: Option<ScratchValue>,
    pub proceeds: Option<SponsorProceeds>,
}

pub struct SponsorProceeds {
    pub quote_delta: ScratchValue,
    pub service_fee: Option<ScratchValue>,
}

/// Sponsor-paid idempotent ATA creates; returns on-chain `ataCost` binding.
pub fn append_sponsor_ata_bootstrap(
    scratch: &mut FrameScratch,
    out: &mut Vec<Instruction>,
    sponsor: Pubkey,
    specs: &[AtaSpec],
) -> Result<Option<ScratchValue>, ScratchError> {
    if specs.is_empty() {
        return Ok(None);
    }

    let mut baseline = scratch.let_builder();
    let mut befores = Vec::with_capacity(specs.len());
    for spec in specs {
        befores.push(baseline.lamports(user_ata(
            &spec.owner,
            &spec.mint,
            &spec.token_program,
        ))?);
    }
    out.push(baseline.build_ix()?);

    for spec in specs {
        out.push(idempotent_ata_create(
            sponsor,
            spec.owner,
            spec.mint,
            spec.token_program,
        ));
    }

    let mut after = scratch.let_builder();
    let mut total = after.let_eval(expr::u64(0))?;
    for (i, spec) in specs.iter().enumerate() {
        let after_lamports = after.lamports(user_ata(
            &spec.owner,
            &spec.mint,
            &spec.token_program,
        ))?;
        let delta = after.let_eval(expr::sub(
            expr::r(&after_lamports),
            expr::r(&befores[i]),
        ))?;
        total = after.let_eval(expr::add(expr::r(&total), expr::r(&delta)))?;
    }
    out.push(after.build_ix()?);
    Ok(Some(total))
}

/// Bind patched repay = (on-chain ataCost + tx fee) × buffer.
pub fn bind_sponsor_repay(
    scratch: &mut FrameScratch,
    out: &mut Vec<Instruction>,
    tx_fee_lamports: u64,
    repay_buffer_percent: u16,
    ata_cost: Option<&ScratchValue>,
) -> Result<ScratchValue, ScratchError> {
    let mut repay_batch = scratch.let_builder();
    let base = if let Some(ata_cost) = ata_cost {
        repay_batch.let_eval(expr::add(
            expr::r(ata_cost),
            expr::u64(tx_fee_lamports),
        ))?
    } else {
        repay_batch.let_eval(expr::u64(tx_fee_lamports))?
    };
    let repay = repay_batch.let_eval(expr::div_floor(
        expr::mul(
            expr::r(&base),
            expr::u64(100 + repay_buffer_percent as u64),
        ),
        expr::u64(100),
    ))?;
    out.push(repay_batch.build_ix()?);
    Ok(repay)
}

pub fn append_sponsor_repay(
    scratch: &mut FrameScratch,
    out: &mut Vec<Instruction>,
    user: Pubkey,
    sponsor: Pubkey,
    params: &SponsorRepayParams,
) -> Result<ScratchValue, ScratchError> {
    let repay = bind_sponsor_repay(
        scratch,
        out,
        params.tx_fee_lamports,
        params.repay_buffer_percent,
        params.ata_cost.as_ref(),
    )?;

    if let Some(proceeds) = &params.proceeds {
        append_proceeds_cover_repay_assert(scratch, out, proceeds, &repay)?;
    }

    append_sponsor_repay_transfer(scratch, out, user, sponsor, &repay)?;
    Ok(repay)
}

pub fn append_sponsor_repay_transfer(
    scratch: &mut FrameScratch,
    out: &mut Vec<Instruction>,
    user: Pubkey,
    sponsor: Pubkey,
    repay: &ScratchValue,
) -> Result<(), ScratchError> {
    let template = system_transfer_template(user, sponsor);
    let built = build_structured_cpi(&template, structured_system_transfer(frame_value(repay)))?;
    out.push(scratch.ix_cpi(&built)?);
    Ok(())
}

pub fn append_proceeds_cover_repay_assert(
    scratch: &mut FrameScratch,
    out: &mut Vec<Instruction>,
    proceeds: &SponsorProceeds,
    repay: &ScratchValue,
) -> Result<(), ScratchError> {
    let required = if let Some(fee) = &proceeds.service_fee {
        expr::add(expr::r(fee), expr::r(repay))
    } else {
        expr::r(repay)
    };
    out.push(scratch.ix_assert(&expr::ge(expr::r(&proceeds.quote_delta), required))?);
    Ok(())
}

/// Scale full-net leg-2 `min_out` by on-chain `leg2_in / net_sol` (sponsor repay reserve).
pub fn bind_sponsored_leg2_min_out(
    scratch: &mut FrameScratch,
    out: &mut Vec<Instruction>,
    full_net_min_out: u64,
    leg2_in: &ScratchValue,
    net_sol: &ScratchValue,
) -> Result<ScratchValue, ScratchError> {
    let mut batch = scratch.let_builder();
    let min_out = batch.let_eval(expr::div_floor(
        expr::mul(
            expr::u64(full_net_min_out),
            expr::r(leg2_in),
        ),
        expr::r(net_sol),
    ))?;
    out.push(batch.build_ix()?);
    Ok(min_out)
}
