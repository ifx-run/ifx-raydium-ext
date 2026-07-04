//! Ifx build — Direct and Bridge transaction assembly.

use crate::close_ata::{build_smart_close_instructions, CloseAtaCandidate};
use crate::pay_asset::SolPayAsset;
use crate::route::{RoutePlan, RouteQuote};
use crate::service_fee::{
    append_proceeds_after_swap, append_quote_proceeds_baseline, idempotent_ata_create,
    static_service_fee_transfer, wsol_proceeds_account, ProceedsLabel,
};
use crate::route::RouteKind;
use crate::sponsor::{
    append_proceeds_cover_repay_assert, append_sponsor_ata_bootstrap,
    append_sponsor_repay_transfer, bind_sponsor_repay, bind_sponsored_leg2_min_out,
    compute_tx_fee_lamports, AtaSpec, SponsorProceeds,
};
use ifx_raydium_config::{AppConfig, PriorityTier};
use ifx_sdk::expr;
use ifx_sdk::core::DEFAULT_TAPE_LEN;
use ifx_sdk::patched_cpi::{build_raw_cpi, raw_cpi_patch};
use ifx_sdk::scratch::FrameScratch;
use ifx_sdk::ScratchError;
use ifx_raydium::constants::NATIVE_MINT;
use ifx_raydium::pool::CpmmPool;
use ifx_raydium::quote::SwapQuote;
use ifx_raydium::swap::{
    close_wsol_ix, direction_for_input, swap_base_input_ix, swap_base_input_template, user_ata,
    AMOUNT_IN_PATCH_OFFSET, MIN_OUT_PATCH_OFFSET,
};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use thiserror::Error;
use tracing::info;

#[derive(Debug, Error)]
pub enum BuildError {
    #[error("scratch: {0}")]
    Scratch(#[from] ScratchError),
    #[error("config: {0}")]
    Config(#[from] ifx_raydium_config::ConfigError),
    #[error("tx: {0}")]
    Tx(String),
}

#[derive(Debug, Clone)]
pub struct BuildPlan {
    pub user: Pubkey,
    pub route: RouteQuote,
    pub service_fee_bps: u16,
    pub fee_recipient: Pubkey,
    pub use_sponsor: bool,
    pub sponsor_pubkey: Option<Pubkey>,
    pub sponsor_repay_buffer_percent: u16,
    pub priority_tier: PriorityTier,
    /// When mint A is WSOL: fund swap from WSOL ATA or native SOL wrap.
    pub sol_pay_asset: Option<SolPayAsset>,
    /// When mint B is WSOL: keep WSOL in ATA or unwrap to native SOL after swap.
    pub sol_receive_asset: Option<SolPayAsset>,
}

#[derive(Debug, Clone)]
pub struct BuildResult {
    pub instructions: Vec<Instruction>,
    /// Optional smart-close tail (appended at finalize when tx size allows).
    pub smart_close_ixs: Vec<Instruction>,
    /// Set when the tx includes Ifx frame instructions; `None` for plain Raydium-only builds.
    pub frame_used: Option<Pubkey>,
    pub route_kind: RouteKind,
}

pub struct Planner {
    config: Arc<AppConfig>,
}

impl Planner {
    pub fn new(config: Arc<AppConfig>) -> Self {
        Self { config }
    }

    pub fn scratch_for_build(&self) -> Result<(FrameScratch, Pubkey), BuildError> {
        let program_id = self.config.ifx_program_id()?;
        let frame_str = self
            .config
            .ifx
            .public_frames
            .first()
            .ok_or_else(|| BuildError::Tx("no public_frames configured".into()))?;
        let frame: Pubkey = frame_str
            .parse()
            .map_err(|_| BuildError::Tx("invalid public frame pubkey".into()))?;
        let scratch = FrameScratch::for_public_frame(frame, Some(DEFAULT_TAPE_LEN), Some(program_id));
        Ok((scratch, frame))
    }

    pub fn build(&self, plan: &BuildPlan) -> Result<BuildResult, BuildError> {
        let mut out = Vec::new();
        out.extend(priority_ixs(self.config.as_ref(), plan.priority_tier));

        let needs_ifx = self.plan_needs_ifx(plan)?;
        let mut scratch_frame = if needs_ifx {
            let (mut scratch, frame) = self.scratch_for_build()?;
            out.push(scratch.ix_reset());
            Some((scratch, frame))
        } else {
            None
        };

        match &plan.route.plan {
            RoutePlan::Direct { pool } => {
                let scratch = scratch_frame.as_mut().map(|(s, _)| s);
                self.build_direct(
                    scratch,
                    &mut out,
                    plan,
                    pool,
                    &plan.route.leg1,
                )?;
            }
            RoutePlan::Bridge { leg1, leg2 } => {
                let leg2_quote = plan.route.leg2.as_ref().ok_or_else(|| {
                    BuildError::Tx("bridge route missing leg2 quote".into())
                })?;
                let (scratch, _) = scratch_frame
                    .as_mut()
                    .expect("bridge route always needs ifx");
                self.build_bridge(
                    scratch,
                    &mut out,
                    plan,
                    leg1,
                    leg2,
                    &plan.route.leg1,
                    leg2_quote,
                )?;
            }
        }

        let mut smart_close_ixs = Vec::new();
        if let Some((scratch, _)) = scratch_frame.as_mut() {
            if let Some(candidate) = input_close_candidate(plan) {
                smart_close_ixs = build_smart_close_instructions(scratch, &[candidate])?;
            }
        }

        let frame_used = scratch_frame.map(|(_, frame)| frame);

        info!(
            route = ?plan.route.kind,
            ix_count = out.len(),
            smart_close_ixs = smart_close_ixs.len(),
            needs_ifx,
            frame = ?frame_used,
            use_sponsor = plan.use_sponsor,
            "tx build complete"
        );

        Ok(BuildResult {
            instructions: out,
            smart_close_ixs,
            frame_used,
            route_kind: plan.route.kind,
        })
    }

    /// Ifx is required for dynamic proceeds fee, patched leg-2 CPI (bridge), sponsor repay, or smart-close.
    fn plan_needs_ifx(&self, plan: &BuildPlan) -> Result<bool, BuildError> {
        match &plan.route.plan {
            RoutePlan::Bridge { .. } => Ok(true),
            RoutePlan::Direct { pool } => {
                let quote = &plan.route.leg1;
                let output_mint = pool
                    .other_mint(&quote.input_mint)
                    .ok_or_else(|| BuildError::Tx("input mint not in pool".into()))?;
                Ok(output_mint.address == NATIVE_MINT
                    || quote.input_mint != NATIVE_MINT
                    || plan.use_sponsor)
            }
        }
    }

    fn sponsor_tx_fee(&self, plan: &BuildPlan) -> u64 {
        compute_tx_fee_lamports(self.config.as_ref(), plan.priority_tier)
    }

    fn build_direct(
        &self,
        mut scratch: Option<&mut FrameScratch>,
        out: &mut Vec<Instruction>,
        plan: &BuildPlan,
        pool: &CpmmPool,
        quote: &SwapQuote,
    ) -> Result<(), BuildError> {
        let output_mint = pool.other_mint(&quote.input_mint).unwrap();
        let output_prog = if pool.token_0_mint.address == output_mint.address {
            pool.token_0_program
        } else {
            pool.token_1_program
        };

        let output_is_sol = output_mint.address == NATIVE_MINT;
        let sponsor = plan.sponsor_pubkey.filter(|_| plan.use_sponsor);
        let mut sponsor_ata_cost = None;

        if let (Some(scratch), Some(sponsor_pk)) = (scratch.as_mut(), sponsor) {
            if output_is_sol {
                sponsor_ata_cost = append_sponsor_ata_bootstrap(
                    scratch,
                    out,
                    sponsor_pk,
                    &direct_sol_output_bootstrap_specs(plan),
                )?;
            } else {
                out.push(idempotent_ata_create(
                    plan.user,
                    plan.user,
                    output_mint.address,
                    output_prog,
                ));
            }
        } else {
            out.push(idempotent_ata_create(
                plan.user,
                plan.user,
                output_mint.address,
                output_prog,
            ));
        }

        let input_is_wsol_mint = quote.input_mint == NATIVE_MINT;
        let sol_pay = plan.sol_pay_asset.unwrap_or(SolPayAsset::NativeSol);
        let pay_native_sol = input_is_wsol_mint && sol_pay == SolPayAsset::NativeSol;

        if input_is_wsol_mint {
            let wsol_ata = user_ata(
                &plan.user,
                &NATIVE_MINT,
                &ifx_raydium::constants::TOKEN_PROGRAM_ID,
            );
            out.push(idempotent_ata_create(
                plan.user,
                plan.user,
                NATIVE_MINT,
                ifx_raydium::constants::TOKEN_PROGRAM_ID,
            ));
            if pay_native_sol {
                out.extend(ifx_raydium::swap::wrap_sol_ixs(
                    &plan.user,
                    wsol_ata,
                    quote.gross_amount_in,
                ));
            }
        }

        if input_is_wsol_mint && quote.platform_fee > 0 {
            let user_wsol_ata = user_ata(
                &plan.user,
                &NATIVE_MINT,
                &ifx_raydium::constants::TOKEN_PROGRAM_ID,
            );
            out.push(static_service_fee_transfer(
                ProceedsLabel::Wsol,
                quote.platform_fee,
                plan.user,
                user_wsol_ata,
                plan.fee_recipient,
                NATIVE_MINT,
                ifx_raydium::constants::TOKEN_PROGRAM_ID,
            )?);
        }

        let swap_amount_in = quote.amount_in;

        let direction = direction_for_input(pool, &quote.input_mint)
            .ok_or_else(|| BuildError::Tx("input mint not in pool".into()))?;

        if output_is_sol {
            let scratch = scratch.ok_or_else(|| {
                BuildError::Tx("SOL output direct route requires ifx scratch".into())
            })?;
            let user_wsol_ata =
                user_ata(&plan.user, &NATIVE_MINT, &ifx_raydium::constants::TOKEN_PROGRAM_ID);
            let proceeds = wsol_proceeds_account(plan.user);
            let before = append_quote_proceeds_baseline(scratch, out, &proceeds)?;
            out.push(swap_base_input_ix(&ifx_raydium::swap::SwapBuildParams {
                pool: pool.clone(),
                user: plan.user,
                direction,
                amount_in: swap_amount_in,
                minimum_amount_out: quote.min_out,
            }));
            let fee_result = append_proceeds_after_swap(
                scratch,
                out,
                &proceeds,
                &before,
                plan.service_fee_bps,
                Some(plan.fee_recipient),
                output_mint.address,
                output_prog,
            )?;
            let receive_native = plan.sol_receive_asset.unwrap_or(SolPayAsset::NativeSol)
                == SolPayAsset::NativeSol;
            if plan.use_sponsor {
                let sponsor_pk = sponsor.ok_or_else(|| {
                    BuildError::Tx("sponsor enabled but pubkey missing".into())
                })?;
                let repay = bind_sponsor_repay(
                    scratch,
                    out,
                    self.sponsor_tx_fee(plan),
                    plan.sponsor_repay_buffer_percent,
                    sponsor_ata_cost.as_ref(),
                )?;
                append_proceeds_cover_repay_assert(
                    scratch,
                    out,
                    &SponsorProceeds {
                        quote_delta: fee_result.quote_delta.clone(),
                        service_fee: fee_result.fee.clone(),
                    },
                    &repay,
                )?;
                out.push(close_wsol_ix(&plan.user, user_wsol_ata));
                append_sponsor_repay_transfer(scratch, out, plan.user, sponsor_pk, &repay)?;
            } else if receive_native {
                out.push(close_wsol_ix(&plan.user, user_wsol_ata));
            }
        } else {
            out.push(swap_base_input_ix(&ifx_raydium::swap::SwapBuildParams {
                pool: pool.clone(),
                user: plan.user,
                direction,
                amount_in: swap_amount_in,
                minimum_amount_out: quote.min_out,
            }));
        }

        Ok(())
    }

    fn build_bridge(
        &self,
        scratch: &mut FrameScratch,
        out: &mut Vec<Instruction>,
        plan: &BuildPlan,
        leg1: &CpmmPool,
        leg2: &CpmmPool,
        leg1_quote: &SwapQuote,
        leg2_quote: &SwapQuote,
    ) -> Result<(), BuildError> {
        let leg2_out = leg2.other_mint(&NATIVE_MINT).unwrap();
        let leg2_out_prog = if leg2.token_0_mint.address == leg2_out.address {
            leg2.token_0_program
        } else {
            leg2.token_1_program
        };

        let sponsor = plan.sponsor_pubkey.filter(|_| plan.use_sponsor);
        let user_wsol_ata =
            user_ata(&plan.user, &NATIVE_MINT, &ifx_raydium::constants::TOKEN_PROGRAM_ID);

        let sponsor_ata_cost = if let Some(sponsor_pk) = sponsor {
            append_sponsor_ata_bootstrap(
                scratch,
                out,
                sponsor_pk,
                &bridge_bootstrap_specs(plan, leg2_out.address, leg2_out_prog),
            )?
        } else {
            out.push(idempotent_ata_create(
                plan.user,
                plan.user,
                NATIVE_MINT,
                ifx_raydium::constants::TOKEN_PROGRAM_ID,
            ));
            out.push(idempotent_ata_create(
                plan.user,
                plan.user,
                leg2_out.address,
                leg2_out_prog,
            ));
            None
        };

        let proceeds = wsol_proceeds_account(plan.user);
        let before = append_quote_proceeds_baseline(scratch, out, &proceeds)?;

        let leg1_dir = direction_for_input(leg1, &leg1_quote.input_mint)
            .ok_or_else(|| BuildError::Tx("leg1 input mint not in pool".into()))?;
        out.push(swap_base_input_ix(&ifx_raydium::swap::SwapBuildParams {
            pool: leg1.clone(),
            user: plan.user,
            direction: leg1_dir,
            amount_in: leg1_quote.amount_in,
            minimum_amount_out: leg1_quote.min_out,
        }));

        let fee_result = append_proceeds_after_swap(
            scratch,
            out,
            &proceeds,
            &before,
            plan.service_fee_bps,
            Some(plan.fee_recipient),
            NATIVE_MINT,
            ifx_raydium::constants::TOKEN_PROGRAM_ID,
        )?;

        let net_sol = fee_result
            .net_quote
            .as_ref()
            .unwrap_or(&fee_result.quote_delta);

        let leg2_dir = direction_for_input(leg2, &NATIVE_MINT)
            .ok_or_else(|| BuildError::Tx("leg2 SOL not in pool".into()))?;

        let hop2_template = swap_base_input_template(&ifx_raydium::swap::SwapBuildParams {
            pool: leg2.clone(),
            user: plan.user,
            direction: leg2_dir,
            amount_in: 0,
            minimum_amount_out: leg2_quote.min_out,
        });

        let mut sponsor_repay = None;
        if plan.use_sponsor {
            let repay = bind_sponsor_repay(
                scratch,
                out,
                self.sponsor_tx_fee(plan),
                plan.sponsor_repay_buffer_percent,
                sponsor_ata_cost.as_ref(),
            )?;
            append_proceeds_cover_repay_assert(
                scratch,
                out,
                &SponsorProceeds {
                    quote_delta: fee_result.quote_delta.clone(),
                    service_fee: fee_result.fee.clone(),
                },
                &repay,
            )?;

            let mut leg2_batch = scratch.let_builder();
            let leg2_in = leg2_batch.let_eval(expr::sub(expr::r(net_sol), expr::r(&repay)))?;
            out.push(leg2_batch.build_ix()?);

            let leg2_min_out = bind_sponsored_leg2_min_out(
                scratch,
                out,
                leg2_quote.min_out,
                &leg2_in,
                net_sol,
            )?;

            let hop2_built = build_raw_cpi(
                &hop2_template,
                &[
                    raw_cpi_patch(AMOUNT_IN_PATCH_OFFSET, &leg2_in),
                    raw_cpi_patch(MIN_OUT_PATCH_OFFSET, &leg2_min_out),
                ],
            )?;
            out.push(scratch.ix_cpi(&hop2_built)?);
            sponsor_repay = Some(repay);
        } else {
            let hop2_built = build_raw_cpi(
                &hop2_template,
                &[raw_cpi_patch(AMOUNT_IN_PATCH_OFFSET, net_sol)],
            )?;
            out.push(scratch.ix_cpi(&hop2_built)?);
        }

        // Bridge WSOL ATA is hop-only — close after leg2 to reclaim rent (unwrap any dust).
        out.push(close_wsol_ix(&plan.user, user_wsol_ata));
        if let Some(repay) = sponsor_repay {
            let sponsor_pk = sponsor.ok_or_else(|| {
                BuildError::Tx("sponsor enabled but pubkey missing".into())
            })?;
            append_sponsor_repay_transfer(scratch, out, plan.user, sponsor_pk, &repay)?;
        }

        Ok(())
    }
}

fn direct_sol_output_bootstrap_specs(plan: &BuildPlan) -> Vec<AtaSpec> {
    let tp = ifx_raydium::constants::TOKEN_PROGRAM_ID;
    vec![AtaSpec {
        owner: plan.user,
        mint: NATIVE_MINT,
        token_program: tp,
    }]
}

fn bridge_bootstrap_specs(plan: &BuildPlan, leg2_mint: Pubkey, leg2_prog: Pubkey) -> Vec<AtaSpec> {
    let tp = ifx_raydium::constants::TOKEN_PROGRAM_ID;
    vec![
        AtaSpec {
            owner: plan.user,
            mint: NATIVE_MINT,
            token_program: tp,
        },
        AtaSpec {
            owner: plan.user,
            mint: leg2_mint,
            token_program: leg2_prog,
        },
    ]
}

fn input_close_candidate(plan: &BuildPlan) -> Option<CloseAtaCandidate> {
    match &plan.route.plan {
        RoutePlan::Direct { pool } => {
            input_ata_close_candidate(plan, pool, plan.route.leg1.input_mint)
        }
        RoutePlan::Bridge { leg1, .. } => {
            input_ata_close_candidate(plan, leg1, plan.route.leg1.input_mint)
        }
    }
}

fn input_ata_close_candidate(
    plan: &BuildPlan,
    pool: &CpmmPool,
    input_mint: Pubkey,
) -> Option<CloseAtaCandidate> {
    if input_mint == NATIVE_MINT {
        return None;
    }
    let token_program = if pool.token_0_mint.address == input_mint {
        pool.token_0_program
    } else if pool.token_1_mint.address == input_mint {
        pool.token_1_program
    } else {
        return None;
    };
    Some(CloseAtaCandidate {
        token_account: user_ata(&plan.user, &input_mint, &token_program),
        rent_destination: plan.user,
        owner: plan.user,
        token_program,
    })
}

fn priority_ixs(config: &AppConfig, tier: PriorityTier) -> Vec<Instruction> {
    let t = config.priority_fee.tier(tier);
    vec![
        ComputeBudgetInstruction::set_compute_unit_limit(t.compute_unit_limit),
        ComputeBudgetInstruction::set_compute_unit_price(t.micro_lamports),
    ]
}
