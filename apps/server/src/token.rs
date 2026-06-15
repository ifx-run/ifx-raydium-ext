//! Mint metadata + wallet balance lookup via RPC.

use crate::amount::format_raw_amount;
use ifx_raydium::constants::NATIVE_MINT;
use ifx_raydium_planner::SolPayAsset;
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use spl_associated_token_account::get_associated_token_address;

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfoRequest {
    pub mint: String,
    #[serde(default)]
    pub user_pubkey: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfoResponse {
    pub mint: String,
    pub label: String,
    pub decimals: u8,
    /// Native wallet SOL (lamports) when mint is SOL; SPL ATA balance otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance_ui: Option<String>,
    /// WSOL ATA balance — only set when mint is native SOL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrapped_balance_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrapped_balance_ui: Option<String>,
}

pub fn mint_label(mint: &Pubkey) -> String {
    if *mint == NATIVE_MINT {
        return "WSOL".into();
    }
    let s = mint.to_string();
    if s == USDC_MINT {
        return "USDC".into();
    }
    if s.len() >= 8 {
        format!("{}…{}", &s[..4], &s[s.len() - 4..])
    } else {
        s
    }
}

pub async fn fetch_token_info(
    rpc_url: &str,
    commitment: CommitmentConfig,
    mint: Pubkey,
    user: Option<Pubkey>,
) -> Result<TokenInfoResponse, String> {
    let rpc = RpcClient::new_with_commitment(rpc_url.to_string(), commitment);

    if mint == NATIVE_MINT {
        let decimals = 9;
        let label = mint_label(&mint);
        let (balance_raw, balance_ui, wrapped_balance_raw, wrapped_balance_ui) = match user {
            Some(u) => {
                let lamports = rpc.get_balance(&u).await.map_err(|e| e.to_string())?;
                let wsol_ata = get_associated_token_address(&u, &NATIVE_MINT);
                let (wsol_raw, wsol_ui) = match rpc.get_token_account_balance(&wsol_ata).await {
                    Ok(bal) => {
                        let raw: u64 = bal.amount.parse().unwrap_or(0);
                        (
                            Some(bal.amount),
                            Some(format_raw_amount(raw, decimals)),
                        )
                    }
                    Err(_) => (Some("0".into()), Some("0".into())),
                };
                (
                    Some(lamports.to_string()),
                    Some(format_raw_amount(lamports, decimals)),
                    wsol_raw,
                    wsol_ui,
                )
            }
            None => (None, None, None, None),
        };
        return Ok(TokenInfoResponse {
            mint: mint.to_string(),
            label,
            decimals,
            balance_raw,
            balance_ui,
            wrapped_balance_raw,
            wrapped_balance_ui,
        });
    }

    let supply = rpc
        .get_token_supply(&mint)
        .await
        .map_err(|e| format!("invalid mint or RPC error: {e}"))?;
    let decimals = supply.decimals;
    let label = mint_label(&mint);

    let (balance_raw, balance_ui) = match user {
        Some(u) => {
            let ata = get_associated_token_address(&u, &mint);
            match rpc.get_token_account_balance(&ata).await {
                Ok(bal) => {
                    let raw: u64 = bal.amount.parse().unwrap_or(0);
                    (
                        Some(bal.amount),
                        Some(format_raw_amount(raw, decimals)),
                    )
                }
                Err(_) => (Some("0".into()), Some("0".into())),
            }
        }
        None => (None, None),
    };

    Ok(TokenInfoResponse {
        mint: mint.to_string(),
        label,
        decimals,
        balance_raw,
        balance_ui,
        wrapped_balance_raw: None,
        wrapped_balance_ui: None,
    })
}

pub async fn validate_sol_pay_balance(
    rpc_url: &str,
    commitment: CommitmentConfig,
    user: Pubkey,
    pay_asset: SolPayAsset,
    amount_in: u64,
    _service_fee_bps: u16,
) -> Result<(), String> {
    const NATIVE_RESERVE: u64 = 10_000_000;

    let info = fetch_token_info(rpc_url, commitment, NATIVE_MINT, Some(user)).await?;

    match pay_asset {
        SolPayAsset::Wsol => {
            let wsol: u64 = info
                .wrapped_balance_raw
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if wsol < amount_in {
                return Err(format!(
                    "insufficient WSOL: need {} WSOL (amount includes platform fee), ATA has {} WSOL — switch pay asset to native SOL or wrap first",
                    format_raw_amount(amount_in, 9),
                    format_raw_amount(wsol, 9),
                ));
            }
        }
        SolPayAsset::NativeSol => {
            let native: u64 = info
                .balance_raw
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let total = amount_in.saturating_add(NATIVE_RESERVE);
            if native < total {
                return Err(format!(
                    "insufficient native SOL: need ~{} SOL (amount includes platform fee + tx reserve), wallet has {} SOL",
                    format_raw_amount(total, 9),
                    format_raw_amount(native, 9),
                ));
            }
        }
    }
    Ok(())
}

pub fn parse_mint_pubkey(mint: &str) -> Result<Pubkey, String> {
    mint.trim()
        .parse()
        .map_err(|_| format!("invalid mint: {mint}"))
}

pub fn parse_user_pubkey(user: Option<&String>) -> Result<Option<Pubkey>, String> {
    match user {
        None => Ok(None),
        Some(s) if s.trim().is_empty() => Ok(None),
        Some(s) => s
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| format!("invalid userPubkey: {s}")),
    }
}
