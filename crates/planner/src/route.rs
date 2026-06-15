//! Parallel Direct vs SOL-bridge routing.

use crate::platform_fee::{
    net_proceeds_after_fee, quote_swap_with_platform_fee, split_gross_input,
};
use ifx_raydium::api::ApiClient;
use ifx_raydium::constants::NATIVE_MINT;
use ifx_raydium::pool::{CpmmPool, PoolRef};
use ifx_raydium::pool_select::{CpmmPoolChoice, PoolSelectError, PoolSelector, SwapPoolSelect};
use ifx_raydium::quote::{quote_swap_base_input, SwapQuote};
use ifx_raydium::rpc::RpcPoolLoader;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::fmt;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum RouteError {
    #[error("pool: {0}")]
    Pool(#[from] PoolSelectError),
    #[error("rpc: {0}")]
    Rpc(#[from] ifx_raydium::rpc::RpcError),
    #[error("no route for {mint_a} -> {mint_b}")]
    NoRoute { mint_a: Pubkey, mint_b: Pubkey },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteKind {
    Direct,
    Bridge,
}

#[derive(Debug, Clone)]
pub enum RoutePlan {
    Direct { pool: CpmmPool },
    Bridge {
        leg1: CpmmPool,
        leg2: CpmmPool,
    },
}

#[derive(Debug, Clone)]
pub struct RouteQuote {
    pub kind: RouteKind,
    pub plan: RoutePlan,
    pub expected_out: u64,
    pub leg1: SwapQuote,
    pub leg2: Option<SwapQuote>,
    /// Shown when Raydium standard pools have much more liquidity than the chosen CPMM pool.
    pub liquidity_note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RouteDecision {
    pub chosen: RouteQuote,
    pub direct: Option<RouteQuote>,
    pub bridge: Option<RouteQuote>,
    pub message: String,
}

pub struct Router {
    pools: PoolSelector,
    rpc: RpcPoolLoader,
}

impl Router {
    pub fn new(api: ApiClient, rpc: RpcPoolLoader) -> Self {
        Self {
            pools: PoolSelector::new(api),
            rpc,
        }
    }

    pub fn with_api_timeout(mut self, secs: u64) -> Self {
        self.pools = self.pools.with_api_timeout(secs);
        self
    }

    pub async fn resolve_direct(
        &self,
        mint_a: Pubkey,
        mint_b: Pubkey,
        amount_in: u64,
        slippage_bps: u16,
        service_fee_bps: u16,
    ) -> Result<Option<RouteQuote>, RouteError> {
        self.try_direct(mint_a, mint_b, amount_in, slippage_bps, service_fee_bps)
            .await
    }

    pub async fn resolve_bridge(
        &self,
        mint_a: Pubkey,
        mint_b: Pubkey,
        amount_in: u64,
        slippage_bps: u16,
        service_fee_bps: u16,
    ) -> Result<Option<RouteQuote>, RouteError> {
        self.try_bridge(mint_a, mint_b, amount_in, slippage_bps, service_fee_bps)
            .await
    }

    pub async fn resolve_forced(
        &self,
        mint_a: Pubkey,
        mint_b: Pubkey,
        amount_in: u64,
        slippage_bps: u16,
        service_fee_bps: u16,
        kind: RouteKind,
    ) -> Result<RouteQuote, RouteError> {
        let quote = match kind {
            RouteKind::Direct => {
                self.resolve_direct(mint_a, mint_b, amount_in, slippage_bps, service_fee_bps)
                    .await?
            }
            RouteKind::Bridge => {
                self.resolve_bridge(mint_a, mint_b, amount_in, slippage_bps, service_fee_bps)
                    .await?
            }
        };
        quote.ok_or(RouteError::NoRoute { mint_a, mint_b })
    }

    pub async fn resolve(
        &self,
        mint_a: Pubkey,
        mint_b: Pubkey,
        amount_in: u64,
        slippage_bps: u16,
        service_fee_bps: u16,
    ) -> Result<RouteDecision, RouteError> {
        info!(
            mint_a = %mint_a,
            mint_b = %mint_b,
            amount_in,
            slippage_bps,
            "route resolve start"
        );

        let direct_fut = self.try_direct(mint_a, mint_b, amount_in, slippage_bps, service_fee_bps);
        let bridge_fut = self.try_bridge(mint_a, mint_b, amount_in, slippage_bps, service_fee_bps);

        let (direct, bridge) = if mint_a == NATIVE_MINT || mint_b == NATIVE_MINT {
            info!("skipping bridge candidate (mint pair includes SOL)");
            (direct_fut.await?, None)
        } else {
            let (d, b) = tokio::join!(direct_fut, bridge_fut);
            (d?, b?)
        };

        debug!(
            direct = direct.is_some(),
            bridge = bridge.is_some(),
            "route candidates evaluated"
        );

        match (&direct, &bridge) {
            (Some(d), None) => Ok(RouteDecision {
                message: with_liquidity_note(
                    format!("Using direct (only candidate) · pool {}", short(&d.plan)),
                    d.liquidity_note.as_deref(),
                ),
                chosen: d.clone(),
                direct: direct.clone(),
                bridge: None,
            }),
            (None, Some(b)) => Ok(RouteDecision {
                message: with_liquidity_note(
                    format!(
                        "Using bridge A→SOL→B · pools {} + {}",
                        short_leg(&b.plan, 1),
                        short_leg(&b.plan, 2)
                    ),
                    b.liquidity_note.as_deref(),
                ),
                chosen: b.clone(),
                direct: None,
                bridge: bridge.clone(),
            }),
            (Some(d), Some(b)) => {
                let tie_margin = (d.expected_out as u128) * 995 / 1000;
                let prefer_direct = (b.expected_out as u128) <= tie_margin;
                let chosen = if prefer_direct { d.clone() } else { b.clone() };
                let msg = if prefer_direct {
                    format!(
                        "comparing direct vs SOL bridge → Using direct (better out / smaller tx) · {}",
                        short(&chosen.plan)
                    )
                } else {
                    format!(
                        "comparing direct vs SOL bridge → Using bridge (better out) · {} + {}",
                        short_leg(&chosen.plan, 1),
                        short_leg(&chosen.plan, 2)
                    )
                };
                info!(
                    route = %chosen.kind,
                    expected_out = chosen.expected_out,
                    direct_out = d.expected_out,
                    bridge_out = b.expected_out,
                    prefer_direct,
                    "route chosen"
                );
                Ok(RouteDecision {
                    message: with_liquidity_note(msg, chosen.liquidity_note.as_deref()),
                    chosen,
                    direct: direct.clone(),
                    bridge: bridge.clone(),
                })
            }
            (None, None) => {
                warn!(
                    mint_a = %mint_a,
                    mint_b = %mint_b,
                    amount_in,
                    "no route: direct and bridge both unavailable"
                );
                Err(RouteError::NoRoute { mint_a, mint_b })
            }
        }
    }

    async fn try_direct(
        &self,
        mint_a: Pubkey,
        mint_b: Pubkey,
        amount_in: u64,
        slippage_bps: u16,
        service_fee_bps: u16,
    ) -> Result<Option<RouteQuote>, RouteError> {
        debug!(mint_a = %mint_a, mint_b = %mint_b, "trying direct route");
        let swap_ctx = swap_pool_select(mint_a, amount_in, slippage_bps, service_fee_bps);
        let choice = match self
            .pools
            .select_cpmm_pool(&self.rpc, &mint_a, &mint_b, Some(swap_ctx))
            .await?
        {
            Some(c) => c,
            None => {
                debug!(mint_a = %mint_a, mint_b = %mint_b, "direct: no cpmm pool found");
                return Ok(None);
            }
        };
        let CpmmPoolChoice {
            pool: hint,
            best_standard_tvl,
        } = choice;
        let liquidity_note = liquidity_note_for(&hint, best_standard_tvl);
        info!(pool_id = %hint.pool_id, "direct: pool selected");
        let pool = self.rpc.hydrate_pool(&hint).await?;
        let leg1 = match quote_swap_with_platform_fee(
            &pool,
            &mint_a,
            amount_in,
            slippage_bps,
            service_fee_bps,
        ) {
            Some(q) => q,
            None => {
                let (r0, r1) = ifx_raydium::rpc::effective_vault_amounts(&pool);
                warn!(
                    pool_id = %pool.pool_id,
                    amount_in,
                    vault_0 = pool.vault_0_amount,
                    vault_1 = pool.vault_1_amount,
                    effective_0 = r0,
                    effective_1 = r1,
                    trade_fee_rate = pool.trade_fee_rate,
                    creator_fee_rate = pool.creator_fee_rate,
                    enable_creator_fee = pool.enable_creator_fee,
                    "direct: quote returned none"
                );
                return Ok(None);
            }
        };
        info!(
            pool_id = %pool.pool_id,
            gross_in = leg1.gross_amount_in,
            platform_fee = leg1.platform_fee,
            swap_in = leg1.amount_in,
            expected_out = leg1.expected_out,
            min_out = leg1.min_out,
            trade_fee_paid = leg1.trade_fee_paid,
            "direct: quote ok"
        );
        Ok(Some(RouteQuote {
            kind: RouteKind::Direct,
            expected_out: leg1.expected_out,
            plan: RoutePlan::Direct { pool },
            leg1,
            leg2: None,
            liquidity_note,
        }))
    }

    async fn try_bridge(
        &self,
        mint_a: Pubkey,
        mint_b: Pubkey,
        amount_in: u64,
        slippage_bps: u16,
        service_fee_bps: u16,
    ) -> Result<Option<RouteQuote>, RouteError> {
        debug!(mint_a = %mint_a, mint_b = %mint_b, "trying bridge route");
        let (leg1_list, leg2_list) = tokio::join!(
            self.pools
                .list_cpmm_candidates(&self.rpc, &mint_a, &NATIVE_MINT),
            self.pools
                .list_cpmm_candidates(&self.rpc, &NATIVE_MINT, &mint_b),
        );
        let leg1_list = leg1_list?;
        let leg2_list = leg2_list?;
        if leg1_list.pools.is_empty() || leg2_list.pools.is_empty() {
            debug!(
                leg1 = leg1_list.pools.len(),
                leg2 = leg2_list.pools.len(),
                "bridge: missing pool on one or both legs"
            );
            return Ok(None);
        }

        let mut cache = PoolHydrateCache::new(&self.rpc);
        let mut best: Option<BridgeCombo> = None;

        for hint1 in &leg1_list.pools {
            let pool1 = match cache.get(hint1).await {
                Ok(p) => p.clone(),
                Err(e) => {
                    warn!(pool_id = %hint1.pool_id, error = %e, leg = 1, "bridge: skip pool");
                    continue;
                }
            };
            let leg1 = match quote_swap_with_platform_fee(
                &pool1,
                &mint_a,
                amount_in,
                slippage_bps,
                service_fee_bps,
            ) {
                Some(q) => q,
                None => continue,
            };
            let leg2_amount_in = net_proceeds_after_fee(leg1.expected_out, service_fee_bps);

            for hint2 in &leg2_list.pools {
                let pool2 = match cache.get(hint2).await {
                    Ok(p) => p.clone(),
                    Err(e) => {
                        warn!(pool_id = %hint2.pool_id, error = %e, leg = 2, "bridge: skip pool");
                        continue;
                    }
                };
                let mut leg2 = match quote_swap_base_input(
                    &pool2,
                    &NATIVE_MINT,
                    leg2_amount_in,
                    slippage_bps,
                ) {
                    Some(q) => q,
                    None => continue,
                };
                leg2.gross_amount_in = leg1.expected_out;
                leg2.platform_fee = leg1.expected_out.saturating_sub(leg2_amount_in);

                let replace = best
                    .as_ref()
                    .map(|b| leg2.expected_out > b.leg2.expected_out)
                    .unwrap_or(true);
                if replace {
                    debug!(
                        leg1_pool = %hint1.pool_id,
                        leg2_pool = %hint2.pool_id,
                        leg1_out = leg1.expected_out,
                        leg2_out = leg2.expected_out,
                        "new best bridge combo"
                    );
                    best = Some(BridgeCombo {
                        pool1: pool1.clone(),
                        pool2: pool2.clone(),
                        hint1: hint1.clone(),
                        hint2: hint2.clone(),
                        leg1: leg1.clone(),
                        leg2,
                    });
                }
            }
        }

        let Some(best) = best else {
            warn!(mint_a = %mint_a, mint_b = %mint_b, "bridge: no quotable pool combination");
            return Ok(None);
        };

        let liquidity_note = liquidity_note_for(&best.hint2, leg2_list.best_standard_tvl)
            .or_else(|| liquidity_note_for(&best.hint1, leg1_list.best_standard_tvl));

        info!(
            leg1_pool = %best.pool1.pool_id,
            leg2_pool = %best.pool2.pool_id,
            leg1_out = best.leg1.expected_out,
            leg2_out = best.leg2.expected_out,
            "bridge: best pool combo selected"
        );

        Ok(Some(RouteQuote {
            kind: RouteKind::Bridge,
            expected_out: best.leg2.expected_out,
            plan: RoutePlan::Bridge {
                leg1: best.pool1,
                leg2: best.pool2,
            },
            leg1: best.leg1,
            leg2: Some(best.leg2),
            liquidity_note,
        }))
    }
}

struct BridgeCombo {
    pool1: CpmmPool,
    pool2: CpmmPool,
    hint1: PoolRef,
    hint2: PoolRef,
    leg1: SwapQuote,
    leg2: SwapQuote,
}

struct PoolHydrateCache<'a> {
    rpc: &'a RpcPoolLoader,
    pools: HashMap<Pubkey, CpmmPool>,
}

impl<'a> PoolHydrateCache<'a> {
    fn new(rpc: &'a RpcPoolLoader) -> Self {
        Self {
            rpc,
            pools: HashMap::new(),
        }
    }

    async fn get(&mut self, hint: &PoolRef) -> Result<&CpmmPool, RouteError> {
        if !self.pools.contains_key(&hint.pool_id) {
            let pool = self.rpc.hydrate_pool(hint).await?;
            self.pools.insert(hint.pool_id, pool);
        }
        Ok(self.pools.get(&hint.pool_id).expect("just inserted"))
    }
}

fn swap_pool_select(
    input_mint: Pubkey,
    gross_amount_in: u64,
    slippage_bps: u16,
    service_fee_bps: u16,
) -> SwapPoolSelect {
    let amount_in = split_gross_input(&input_mint, gross_amount_in, service_fee_bps)
        .map(|s| s.swap_in)
        .unwrap_or(gross_amount_in);
    SwapPoolSelect {
        input_mint,
        amount_in,
        slippage_bps,
    }
}

fn liquidity_note_for(pool: &ifx_raydium::pool::PoolRef, best_standard_tvl: Option<f64>) -> Option<String> {
    let standard = best_standard_tvl?;
    let cpmm_tvl = pool.tvl.unwrap_or(0.0);
    if standard > cpmm_tvl * 5.0 && standard > 10_000.0 {
        Some(format!(
            "Raydium site may use standard AMM (~${standard:.0} TVL); this app routes CPMM only (~${cpmm_tvl:.0})"
        ))
    } else {
        None
    }
}

fn with_liquidity_note(message: String, note: Option<&str>) -> String {
    match note {
        Some(n) => format!("{message} · {n}"),
        None => message,
    }
}

fn short(plan: &RoutePlan) -> String {
    match plan {
        RoutePlan::Direct { pool } => short_pk(&pool.pool_id),
        RoutePlan::Bridge { leg1, .. } => short_pk(&leg1.pool_id),
    }
}

fn short_leg(plan: &RoutePlan, leg: u8) -> String {
    match plan {
        RoutePlan::Direct { pool } => short_pk(&pool.pool_id),
        RoutePlan::Bridge { leg1, leg2 } => {
            if leg == 1 {
                short_pk(&leg1.pool_id)
            } else {
                short_pk(&leg2.pool_id)
            }
        }
    }
}

fn short_pk(pk: &Pubkey) -> String {
    let s = pk.to_string();
    format!("{}…{}", &s[..4], &s[s.len() - 4..])
}

impl fmt::Display for RouteKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RouteKind::Direct => write!(f, "Direct"),
            RouteKind::Bridge => write!(f, "Bridge"),
        }
    }
}
