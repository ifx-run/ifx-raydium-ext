//! Application configuration (TOML).

use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid pubkey {field}: {value}")]
    InvalidPubkey { field: &'static str, value: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub solana: SolanaConfig,
    pub ifx: IfxConfig,
    #[serde(default)]
    pub priority_fee: PriorityFeeConfig,
    pub service_fee: ServiceFeeConfig,
    #[serde(default)]
    pub sponsor: SponsorConfig,
    #[serde(default)]
    pub quote: QuoteConfig,
    #[serde(default)]
    pub wallet: WalletConfig,
    #[serde(default)]
    pub trade: TradeConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub network: NetworkConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SolanaConfig {
    pub rpc_url: String,
    #[serde(default = "default_commitment")]
    pub commitment: String,
    #[serde(default)]
    pub address_lookup_tables: Vec<String>,
}

fn default_commitment() -> String {
    "confirmed".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct IfxConfig {
    pub program_id: String,
    pub public_frames: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PriorityTier {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PriorityFeeConfig {
    #[serde(default)]
    pub default_tier: PriorityTier,
    #[serde(default = "default_low_fee")]
    pub low: FeeTierValues,
    #[serde(default = "default_medium_fee")]
    pub medium: FeeTierValues,
    #[serde(default = "default_high_fee")]
    pub high: FeeTierValues,
}

impl Default for PriorityFeeConfig {
    fn default() -> Self {
        Self {
            default_tier: PriorityTier::default(),
            low: default_low_fee(),
            medium: default_medium_fee(),
            high: default_high_fee(),
        }
    }
}

fn default_low_fee() -> FeeTierValues {
    FeeTierValues {
        micro_lamports: 1_000,
        compute_unit_limit: 400_000,
    }
}

fn default_medium_fee() -> FeeTierValues {
    FeeTierValues {
        micro_lamports: 10_000,
        compute_unit_limit: 500_000,
    }
}

fn default_high_fee() -> FeeTierValues {
    FeeTierValues {
        micro_lamports: 50_000,
        compute_unit_limit: 600_000,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeeTierValues {
    pub micro_lamports: u64,
    pub compute_unit_limit: u32,
}

impl PriorityFeeConfig {
    pub fn tier(&self, tier: PriorityTier) -> &FeeTierValues {
        match tier {
            PriorityTier::Low => &self.low,
            PriorityTier::Medium => &self.medium,
            PriorityTier::High => &self.high,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceFeeConfig {
    pub bps: u16,
    pub pubkey: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SponsorConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub pubkey: String,
    #[serde(default)]
    pub keypair_path: Option<String>,
    #[serde(default = "default_repay_buffer")]
    pub repay_buffer_percent: u16,
}

fn default_repay_buffer() -> u16 {
    10
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuoteConfig {
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    #[serde(default = "default_slippage_bps")]
    pub default_slippage_bps: u16,
}

fn default_debounce_ms() -> u64 {
    300
}

fn default_slippage_bps() -> u16 {
    100
}

fn default_mint_a() -> String {
    "So11111111111111111111111111111111111111112".to_string()
}

fn default_mint_b() -> String {
    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string()
}

fn default_trade_amount() -> String {
    "0.01".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct WalletConfig {
    /// Solana JSON keypair file; required to sign and send transactions.
    #[serde(default)]
    pub keypair_path: Option<String>,
    /// Optional override; if omitted and keypair_path is set, derived from the keypair.
    #[serde(default)]
    pub pubkey: Option<String>,
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self {
            keypair_path: None,
            pubkey: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TradeConfig {
    #[serde(default = "default_mint_a")]
    pub mint_a: String,
    #[serde(default = "default_mint_b")]
    pub mint_b: String,
    #[serde(default = "default_trade_amount")]
    pub amount: String,
    #[serde(default = "default_slippage_bps")]
    pub slippage_bps: u16,
    #[serde(default)]
    pub sponsored: bool,
}

impl Default for TradeConfig {
    fn default() -> Self {
        Self {
            mint_a: default_mint_a(),
            mint_b: default_mint_b(),
            amount: default_trade_amount(),
            slippage_bps: default_slippage_bps(),
            sponsored: false,
        }
    }
}

impl Default for SolanaConfig {
    fn default() -> Self {
        Self {
            rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
            commitment: default_commitment(),
            address_lookup_tables: vec![],
        }
    }
}

impl Default for SponsorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            pubkey: String::new(),
            keypair_path: None,
            repay_buffer_percent: default_repay_buffer(),
        }
    }
}

impl Default for QuoteConfig {
    fn default() -> Self {
        Self {
            debounce_ms: default_debounce_ms(),
            default_slippage_bps: default_slippage_bps(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_server_host")]
    pub host: String,
    #[serde(default = "default_server_port")]
    pub port: u16,
}

fn default_server_host() -> String {
    "127.0.0.1".to_string()
}

fn default_server_port() -> u16 {
    8788
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_server_host(),
            port: default_server_port(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    /// HTTP(S) proxy for Raydium API (`api-v3.raydium.io`). When unset, `HTTPS_PROXY` /
    /// `HTTP_PROXY` / `ALL_PROXY` environment variables are used if present.
    #[serde(default)]
    pub http_proxy: Option<String>,
    #[serde(default = "default_raydium_api_timeout_secs")]
    pub raydium_api_timeout_secs: u64,
}

fn default_raydium_api_timeout_secs() -> u64 {
    12
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            http_proxy: None,
            raydium_api_timeout_secs: default_raydium_api_timeout_secs(),
        }
    }
}

impl NetworkConfig {
    /// Config file value, else standard proxy env vars.
    pub fn effective_http_proxy(&self) -> Option<String> {
        if let Some(p) = self.http_proxy.as_ref().filter(|s| !s.trim().is_empty()) {
            return Some(p.trim().to_string());
        }
        ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
            .into_iter()
            .find_map(|key| std::env::var(key).ok())
            .filter(|s| !s.trim().is_empty())
    }
}

impl AppConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn parse_pubkey(&self, field: &'static str, value: &str) -> Result<Pubkey, ConfigError> {
        value
            .parse()
            .map_err(|_| ConfigError::InvalidPubkey {
                field,
                value: value.to_string(),
            })
    }

    pub fn ifx_program_id(&self) -> Result<Pubkey, ConfigError> {
        self.parse_pubkey("ifx.program_id", &self.ifx.program_id)
    }

    pub fn service_fee_pubkey(&self) -> Result<Pubkey, ConfigError> {
        self.parse_pubkey("service_fee.pubkey", &self.service_fee.pubkey)
    }
}
