//! Sponsor tx fee estimation (ported from pumpfun-ext `sponsor/fees.ts`).

use ifx_raydium_config::{AppConfig, PriorityTier};

/// Lamports charged per Ed25519 signature (base fee, pre-priority).
pub const LAMPORTS_PER_SIGNATURE: u64 = 5_000;

/// Sponsor co-signs as fee payer → user + sponsor.
pub fn sponsor_tx_signature_count() -> u64 {
    2
}

/// v1 `priority_fee` (total lamports) equivalent of v0 micro-lamports-per-CU.
pub fn priority_fee_lamports_for_tier(config: &AppConfig, tier: PriorityTier) -> u64 {
    let t = config.priority_fee.tier(tier);
    (t.compute_unit_limit as u64 * t.micro_lamports).div_ceil(1_000_000)
}

/// Exact tx fee budget: base signatures + priority fee ceiling
/// (`compute_unit_limit × micro_lamports / 1_000_000`).
pub fn compute_tx_fee_lamports(config: &AppConfig, tier: PriorityTier) -> u64 {
    let base = sponsor_tx_signature_count() * LAMPORTS_PER_SIGNATURE;
    base + priority_fee_lamports_for_tier(config, tier)
}

pub fn apply_repay_buffer(settle: u64, buffer_percent: u16) -> u64 {
    settle * (100 + buffer_percent as u64) / 100
}
