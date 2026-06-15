//! Raydium API v3 client.

use crate::constants::{AMM_V4_PROGRAM_ID, API_V3_BASE, CPMM_PROGRAM_ID};
use crate::pool::{MintInfo, PoolRef};
use reqwest::Client;
use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct HttpClientOptions {
    pub connect_timeout: Duration,
    pub timeout: Duration,
    pub proxy: Option<String>,
}

impl Default for HttpClientOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            timeout: Duration::from_secs(12),
            proxy: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("api: {0}")]
    Api(String),
    #[error("invalid pubkey: {0}")]
    InvalidPubkey(String),
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    success: bool,
    #[serde(default)]
    msg: Option<String>,
    data: T,
}

#[derive(Debug, Deserialize)]
struct PoolListData {
    data: Vec<ApiPool>,
}

#[derive(Debug, Deserialize)]
struct ApiPool {
    id: String,
    #[serde(rename = "programId")]
    program_id: String,
    #[serde(rename = "mintA")]
    mint_a: ApiMint,
    #[serde(rename = "mintB")]
    mint_b: ApiMint,
    #[serde(default)]
    #[serde(rename = "marketId")]
    market_id: Option<String>,
    #[serde(default)]
    pooltype: Vec<String>,
    #[serde(default)]
    tvl: Option<f64>,
    #[serde(default)]
    config: Option<ApiPoolConfig>,
}

#[derive(Debug, Deserialize)]
struct ApiMint {
    address: String,
    #[serde(rename = "programId")]
    program_id: String,
    decimals: u8,
    #[serde(default)]
    symbol: String,
}

#[derive(Debug, Deserialize)]
struct ApiPoolConfig {
    id: String,
    #[serde(rename = "tradeFeeRate")]
    trade_fee_rate: u64,
}

pub struct ApiClient {
    http: reqwest::Client,
    base: String,
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::with_options(API_V3_BASE, HttpClientOptions::default())
            .expect("default raydium http client")
    }
}

impl ApiClient {
    pub fn new(base: impl Into<String>) -> Self {
        Self::with_options(base, HttpClientOptions::default())
            .expect("raydium http client")
    }

    pub fn with_timeout(base: impl Into<String>, timeout: Duration) -> Self {
        Self::with_options(
            base,
            HttpClientOptions {
                timeout,
                ..HttpClientOptions::default()
            },
        )
        .expect("raydium http client")
    }

    pub fn with_options(base: impl Into<String>, opts: HttpClientOptions) -> Result<Self, ApiError> {
        let mut builder = Client::builder()
            .connect_timeout(opts.connect_timeout)
            .timeout(opts.timeout);

        if let Some(proxy_url) = opts.proxy.filter(|s| !s.trim().is_empty()) {
            let proxy = reqwest::Proxy::all(proxy_url.trim())
                .map_err(|e| ApiError::Api(format!("invalid http proxy: {e}")))?;
            builder = builder.proxy(proxy);
            info!(proxy = %mask_proxy_url(proxy_url.trim()), "raydium api http proxy configured");
        }

        let http = builder.build()?;
        Ok(Self {
            http,
            base: base.into(),
        })
    }

    /// All CPMM pools for a mint pair (sorted by API TVL desc) plus best non-CPMM TVL on the pair.
    pub async fn list_cpmm_pools(
        &self,
        mint_a: &Pubkey,
        mint_b: &Pubkey,
    ) -> Result<(Vec<PoolRef>, Option<f64>), ApiError> {
        let pools = self.fetch_pools_by_mints(mint_a, mint_b).await?;
        let best_standard_tvl = pools
            .iter()
            .filter(|p| !is_cpmm_pool(p))
            .map(|p| p.tvl.unwrap_or(0.0))
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .filter(|tvl| *tvl > 0.0);

        let mut cpmm: Vec<PoolRef> = pools
            .into_iter()
            .filter(|p| is_cpmm_pool(p))
            .map(|p| api_pool_to_ref(&p))
            .collect::<Result<_, _>>()?;
        cpmm.sort_by(|a, b| {
            b.tvl
                .unwrap_or(0.0)
                .partial_cmp(&a.tvl.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        debug!(
            cpmm_count = cpmm.len(),
            ?best_standard_tvl,
            mint_a = %mint_a,
            mint_b = %mint_b,
            "raydium api cpmm pool list"
        );
        Ok((cpmm, best_standard_tvl))
    }

    pub async fn select_cpmm_pool(
        &self,
        mint_a: &Pubkey,
        mint_b: &Pubkey,
    ) -> Result<Option<PoolRef>, ApiError> {
        Ok(self.list_cpmm_pools(mint_a, mint_b).await?.0.into_iter().next())
    }

    async fn fetch_pools_by_mints(
        &self,
        mint_a: &Pubkey,
        mint_b: &Pubkey,
    ) -> Result<Vec<ApiPool>, ApiError> {
        let (mint1, mint2) = sort_mints(mint_a, mint_b);
        let url = format!(
            "{}/pools/info/mint?mint1={}&mint2={}&poolType=all&poolSortField=liquidity&sortType=desc&pageSize=50&page=1",
            self.base, mint1, mint2
        );
        debug!(%url, "raydium api pool query");
        let envelope: ApiEnvelope<PoolListData> = self.http.get(url).send().await?.json().await?;
        if !envelope.success {
            let msg = envelope.msg.unwrap_or_else(|| "pool query failed".into());
            warn!(error = %msg, mint1 = %mint1, mint2 = %mint2, "raydium api pool query rejected");
            return Err(ApiError::Api(msg));
        }

        debug!(
            total = envelope.data.data.len(),
            mint1 = %mint1,
            mint2 = %mint2,
            "raydium api pool query response"
        );
        Ok(envelope.data.data)
    }
}

fn sort_mints(a: &Pubkey, b: &Pubkey) -> (Pubkey, Pubkey) {
    if a.to_bytes() < b.to_bytes() {
        (*a, *b)
    } else {
        (*b, *a)
    }
}

fn is_cpmm_pool(pool: &ApiPool) -> bool {
    if pool.program_id != CPMM_PROGRAM_ID.to_string() {
        return false;
    }
    if pool.market_id.is_some() {
        return false;
    }
    if pool.program_id == AMM_V4_PROGRAM_ID.to_string() {
        return false;
    }
    pool.pooltype.iter().any(|t| t.eq_ignore_ascii_case("cpmm"))
}

fn api_pool_to_ref(pool: &ApiPool) -> Result<PoolRef, ApiError> {
    let pool_id = pool
        .id
        .parse()
        .map_err(|_| ApiError::InvalidPubkey(pool.id.clone()))?;
    let amm_config = pool
        .config
        .as_ref()
        .map(|c| c.id.parse())
        .transpose()
        .map_err(|_| ApiError::InvalidPubkey("config.id".into()))?;

    Ok(PoolRef {
        pool_id,
        amm_config,
        mint_a: parse_mint(&pool.mint_a)?,
        mint_b: parse_mint(&pool.mint_b)?,
        trade_fee_rate: pool.config.as_ref().map(|c| c.trade_fee_rate),
        tvl: pool.tvl,
    })
}

fn parse_mint(m: &ApiMint) -> Result<MintInfo, ApiError> {
    Ok(MintInfo {
        address: m
            .address
            .parse()
            .map_err(|_| ApiError::InvalidPubkey(m.address.clone()))?,
        token_program: m
            .program_id
            .parse()
            .map_err(|_| ApiError::InvalidPubkey(m.program_id.clone()))?,
        decimals: m.decimals,
        symbol: m.symbol.clone(),
    })
}

fn mask_proxy_url(url: &str) -> String {
    if let Some(at) = url.rfind('@') {
        format!("***@{}", &url[at + 1..])
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NATIVE_MINT;

    #[test]
    fn sort_mints_byte_order() {
        let usdc: Pubkey = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".parse().unwrap();
        let sol = NATIVE_MINT;
        let (m1, m2) = sort_mints(&sol, &usdc);
        assert!(m1.to_bytes() < m2.to_bytes());
    }
}
