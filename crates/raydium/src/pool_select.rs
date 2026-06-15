//! Pool selection — Raydium API first, RPC on-chain discovery as fallback.

use crate::api::{ApiClient, ApiError};
use crate::pool::PoolRef;
use crate::quote::quote_swap_base_input;
use crate::rpc::{effective_vault_amounts, RpcPoolLoader};
use solana_sdk::pubkey::Pubkey;
use std::time::Duration;
use thiserror::Error;
use tokio::time::timeout;
use tracing::{debug, info, warn};

const MAX_POOL_CANDIDATES: usize = 8;
/// RPC `getProgramAccounts` order is undefined — scan more before trimming.
const MAX_RPC_POOL_SCAN: usize = 32;

#[derive(Debug, Error)]
pub enum PoolSelectError {
    #[error("api: {0}")]
    Api(#[from] ApiError),
    #[error("rpc: {0}")]
    Rpc(#[from] crate::rpc::RpcError),
}

/// When set, candidates are hydrated and the pool with the best on-chain quote wins.
#[derive(Debug, Clone, Copy)]
pub struct SwapPoolSelect {
    pub input_mint: Pubkey,
    pub amount_in: u64,
    pub slippage_bps: u16,
}

#[derive(Debug, Clone)]
pub struct CpmmPoolChoice {
    pub pool: PoolRef,
    /// Best Raydium standard (AMM v4 / CLMM) TVL for this pair from API, if known.
    pub best_standard_tvl: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct PoolCandidateList {
    pub pools: Vec<PoolRef>,
    pub best_standard_tvl: Option<f64>,
    pub source: &'static str,
}

pub struct PoolSelector {
    api: ApiClient,
    api_timeout: Duration,
}

impl PoolSelector {
    pub fn new(api: ApiClient) -> Self {
        Self {
            api,
            api_timeout: Duration::from_secs(12),
        }
    }

    pub fn with_api_timeout(mut self, secs: u64) -> Self {
        self.api_timeout = Duration::from_secs(secs.max(1));
        self
    }

    /// CPMM candidates for a mint pair, ranked and capped for quoting.
    pub async fn list_cpmm_candidates(
        &self,
        rpc: &RpcPoolLoader,
        mint_a: &Pubkey,
        mint_b: &Pubkey,
    ) -> Result<PoolCandidateList, PoolSelectError> {
        let (candidates, best_standard_tvl, source) =
            self.fetch_candidates(rpc, mint_a, mint_b).await?;
        let pools = self
            .rank_and_limit_candidates(rpc, &candidates, mint_a, mint_b, source)
            .await?;
        Ok(PoolCandidateList {
            pools,
            best_standard_tvl,
            source,
        })
    }

    pub async fn select_cpmm_pool(
        &self,
        rpc: &RpcPoolLoader,
        mint_a: &Pubkey,
        mint_b: &Pubkey,
        swap: Option<SwapPoolSelect>,
    ) -> Result<Option<CpmmPoolChoice>, PoolSelectError> {
        debug!(mint_a = %mint_a, mint_b = %mint_b, has_swap = swap.is_some(), "selecting cpmm pool");
        let list = self.list_cpmm_candidates(rpc, mint_a, mint_b).await?;
        if list.pools.is_empty() {
            warn!(mint_a = %mint_a, mint_b = %mint_b, "no cpmm pool via api or rpc");
            return Ok(None);
        }

        let pool = if let Some(ctx) = swap {
            self.pick_best_by_quote(rpc, &list.pools, ctx).await?
        } else {
            list.pools.into_iter().next()
        };

        let Some(pool) = pool else {
            warn!(mint_a = %mint_a, mint_b = %mint_b, "no quotable cpmm pool among candidates");
            return Ok(None);
        };

        info!(
            pool_id = %pool.pool_id,
            source = list.source,
            tvl = ?pool.tvl,
            best_standard_tvl = ?list.best_standard_tvl,
            "cpmm pool selected"
        );
        Ok(Some(CpmmPoolChoice {
            pool,
            best_standard_tvl: list.best_standard_tvl,
        }))
    }

    async fn fetch_candidates(
        &self,
        rpc: &RpcPoolLoader,
        mint_a: &Pubkey,
        mint_b: &Pubkey,
    ) -> Result<(Vec<PoolRef>, Option<f64>, &'static str), PoolSelectError> {
        match timeout(
            self.api_timeout,
            self.api.list_cpmm_pools(mint_a, mint_b),
        )
        .await
        {
            Ok(Ok((pools, best_standard_tvl))) if !pools.is_empty() => {
                return Ok((pools, best_standard_tvl, "api"));
            }
            Ok(Ok(_)) => {
                warn!(mint_a = %mint_a, mint_b = %mint_b, "api returned no cpmm pool, trying rpc");
            }
            Ok(Err(e)) => {
                warn!(error = %e, mint_a = %mint_a, mint_b = %mint_b, "api pool query failed, trying rpc");
            }
            Err(_) => {
                warn!(
                    timeout_secs = self.api_timeout.as_secs(),
                    mint_a = %mint_a,
                    mint_b = %mint_b,
                    "api pool query timed out, trying rpc"
                );
            }
        }

        let pools = rpc.find_all_cpmm_pools(mint_a, mint_b).await?;
        Ok((pools, None, "rpc"))
    }

    async fn rank_and_limit_candidates(
        &self,
        rpc: &RpcPoolLoader,
        candidates: &[PoolRef],
        mint_a: &Pubkey,
        mint_b: &Pubkey,
        source: &'static str,
    ) -> Result<Vec<PoolRef>, PoolSelectError> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        if source == "api" {
            return Ok(candidates.iter().take(MAX_POOL_CANDIDATES).cloned().collect());
        }

        let scan_limit = candidates.len().min(MAX_RPC_POOL_SCAN);
        let mut scored: Vec<(PoolRef, u128)> = Vec::with_capacity(scan_limit);
        for hint in candidates.iter().take(scan_limit) {
            let pool = match rpc.hydrate_pool(hint).await {
                Ok(p) => p,
                Err(e) => {
                    warn!(pool_id = %hint.pool_id, error = %e, "skip pool: hydrate failed during rank");
                    continue;
                }
            };
            let (r0, r1) = effective_vault_amounts(&pool);
            let score = match (pool.side_of(mint_a), pool.side_of(mint_b)) {
                (Some(_), Some(_)) => (r0 as u128).saturating_mul(r1 as u128),
                _ => 0,
            };
            if score > 0 {
                scored.push((hint.clone(), score));
            }
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        debug!(
            source = "rpc",
            scanned = scan_limit,
            ranked = scored.len(),
            "rpc cpmm candidates ranked by reserves"
        );
        Ok(scored
            .into_iter()
            .take(MAX_POOL_CANDIDATES)
            .map(|(p, _)| p)
            .collect())
    }

    async fn pick_best_by_quote(
        &self,
        rpc: &RpcPoolLoader,
        candidates: &[PoolRef],
        ctx: SwapPoolSelect,
    ) -> Result<Option<PoolRef>, PoolSelectError> {
        let mut best: Option<(PoolRef, u64)> = None;
        for hint in candidates {
            let pool = match rpc.hydrate_pool(hint).await {
                Ok(p) => p,
                Err(e) => {
                    warn!(pool_id = %hint.pool_id, error = %e, "skip pool: hydrate failed");
                    continue;
                }
            };
            let Some(quote) =
                quote_swap_base_input(&pool, &ctx.input_mint, ctx.amount_in, ctx.slippage_bps)
            else {
                warn!(pool_id = %hint.pool_id, "skip pool: quote returned none");
                continue;
            };
            let replace = best
                .as_ref()
                .map(|(_, out)| quote.expected_out > *out)
                .unwrap_or(true);
            if replace {
                debug!(
                    pool_id = %hint.pool_id,
                    expected_out = quote.expected_out,
                    "new best cpmm quote"
                );
                best = Some((hint.clone(), quote.expected_out));
            }
        }
        Ok(best.map(|(pool, expected_out)| {
            info!(pool_id = %pool.pool_id, expected_out, "selected cpmm pool by quote");
            pool
        }))
    }
}
