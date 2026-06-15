//! On-chain pool state loading via RPC.

use crate::constants::{AUTH_SEED, CPMM_PROGRAM_ID};
use crate::pool::{CpmmPool, MintInfo, PoolRef};
use borsh::BorshDeserialize;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::account::Account;
use solana_sdk::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use spl_token_interface::state::Account as TokenAccount;
use thiserror::Error;
use tracing::{debug, warn};

#[derive(Debug, Error)]
pub enum RpcError {
    #[error("rpc: {0}")]
    Client(#[from] solana_client::client_error::ClientError),
    #[error("account missing: {0}")]
    MissingAccount(Pubkey),
    #[error("decode: {0}")]
    Decode(String),
}

pub struct RpcPoolLoader {
    rpc: RpcClient,
}

impl RpcPoolLoader {
    pub fn new(rpc_url: impl Into<String>, commitment: CommitmentConfig) -> Self {
        Self {
            rpc: RpcClient::new_with_commitment(rpc_url.into(), commitment),
        }
    }

    pub fn client(&self) -> &RpcClient {
        &self.rpc
    }

    pub async fn hydrate_pool(&self, hint: &PoolRef) -> Result<CpmmPool, RpcError> {
        let pool_account = self.rpc.get_account(&hint.pool_id).await?;
        let pool_state = decode_pool_state(&pool_account)?;

        let amm_config_key = pool_state.amm_config;
        let amm_config_account = self.rpc.get_account(&amm_config_key).await?;
        let amm_config = decode_amm_config(&amm_config_account)?;

        let vault_accounts = self
            .rpc
            .get_multiple_accounts(&[pool_state.token_0_vault, pool_state.token_1_vault])
            .await?;

        let vault0 = vault_accounts
            .first()
            .and_then(|a| a.as_ref())
            .ok_or(RpcError::MissingAccount(pool_state.token_0_vault))?;
        let vault1 = vault_accounts
            .get(1)
            .and_then(|a| a.as_ref())
            .ok_or(RpcError::MissingAccount(pool_state.token_1_vault))?;

        let vault_0_amount = token_amount(vault0)?;
        let vault_1_amount = token_amount(vault1)?;

        let (authority, _) =
            Pubkey::find_program_address(&[AUTH_SEED], &CPMM_PROGRAM_ID);

        let token_0_mint = mint_info_from_pool(&hint, &pool_state.token_0_mint, pool_state.mint_0_decimals);
        let token_1_mint = mint_info_from_pool(&hint, &pool_state.token_1_mint, pool_state.mint_1_decimals);

        let (effective_0, effective_1) = {
            let fees_0 = pool_state
                .protocol_fees_token_0
                .saturating_add(pool_state.fund_fees_token_0)
                .saturating_add(pool_state.creator_fees_token_0);
            let fees_1 = pool_state
                .protocol_fees_token_1
                .saturating_add(pool_state.fund_fees_token_1)
                .saturating_add(pool_state.creator_fees_token_1);
            (
                vault_0_amount.saturating_sub(fees_0),
                vault_1_amount.saturating_sub(fees_1),
            )
        };

        debug!(
            pool_id = %hint.pool_id,
            vault_0 = vault_0_amount,
            vault_1 = vault_1_amount,
            effective_0,
            effective_1,
            protocol_fees_0 = pool_state.protocol_fees_token_0,
            protocol_fees_1 = pool_state.protocol_fees_token_1,
            fund_fees_0 = pool_state.fund_fees_token_0,
            fund_fees_1 = pool_state.fund_fees_token_1,
            creator_fees_0 = pool_state.creator_fees_token_0,
            creator_fees_1 = pool_state.creator_fees_token_1,
            trade_fee_rate = amm_config.trade_fee_rate,
            creator_fee_rate = amm_config.creator_fee_rate,
            enable_creator_fee = pool_state.enable_creator_fee,
            creator_fee_on = pool_state.creator_fee_on,
            "pool hydrated"
        );

        if effective_0 == 0 || effective_1 == 0 {
            warn!(
                pool_id = %hint.pool_id,
                effective_0,
                effective_1,
                vault_0 = vault_0_amount,
                vault_1 = vault_1_amount,
                "pool effective reserves are zero"
            );
        }

        Ok(CpmmPool {
            pool_id: hint.pool_id,
            amm_config: amm_config_key,
            authority,
            observation_state: pool_state.observation_key,
            token_0_mint,
            token_1_mint,
            token_0_vault: pool_state.token_0_vault,
            token_1_vault: pool_state.token_1_vault,
            token_0_program: pool_state.token_0_program,
            token_1_program: pool_state.token_1_program,
            trade_fee_rate: amm_config.trade_fee_rate,
            creator_fee_rate: amm_config.creator_fee_rate,
            enable_creator_fee: pool_state.enable_creator_fee,
            creator_fee_on: pool_state.creator_fee_on,
            protocol_fees_token_0: pool_state.protocol_fees_token_0,
            protocol_fees_token_1: pool_state.protocol_fees_token_1,
            fund_fees_token_0: pool_state.fund_fees_token_0,
            fund_fees_token_1: pool_state.fund_fees_token_1,
            creator_fees_token_0: pool_state.creator_fees_token_0,
            creator_fees_token_1: pool_state.creator_fees_token_1,
            vault_0_amount,
            vault_1_amount,
        })
    }
}

fn mint_info_from_pool(hint: &PoolRef, mint: &Pubkey, decimals: u8) -> MintInfo {
    if hint.mint_a.address == *mint {
        return hint.mint_a.clone();
    }
    if hint.mint_b.address == *mint {
        return hint.mint_b.clone();
    }
    MintInfo {
        address: *mint,
        token_program: crate::constants::TOKEN_PROGRAM_ID,
        decimals,
        symbol: String::new(),
    }
}

fn token_amount(account: &Account) -> Result<u64, RpcError> {
    if account.data.len() == TokenAccount::LEN {
        let token = TokenAccount::unpack(&account.data)
            .map_err(|e| RpcError::Decode(e.to_string()))?;
        return Ok(token.amount);
    }
    // Token-2022 accounts are longer; amount is still at SPL layout offset 64..72.
    if account.data.len() >= 72 {
        let amount = u64::from_le_bytes(account.data[64..72].try_into().unwrap());
        return Ok(amount);
    }
    Err(RpcError::Decode("token account too short".into()))
}

#[derive(Debug, Clone, BorshDeserialize)]
struct AmmConfig {
    _bump: u8,
    _disable_create_pool: bool,
    _index: u16,
    pub trade_fee_rate: u64,
    _protocol_fee_rate: u64,
    _fund_fee_rate: u64,
    _create_pool_fee: u64,
    _protocol_owner: Pubkey,
    _fund_owner: Pubkey,
    pub creator_fee_rate: u64,
    _padding: [u64; 15],
}

fn decode_amm_config(account: &Account) -> Result<AmmConfig, RpcError> {
    if account.data.len() < 8 {
        return Err(RpcError::Decode("amm_config too short".into()));
    }
    AmmConfig::try_from_slice(&account.data[8..])
        .map_err(|e| RpcError::Decode(e.to_string()))
}

/// Zero-copy `PoolState` layout (packed) — mirror of Raydium CPMM on-chain.
#[derive(Debug, Clone, Copy)]
struct PoolStateRaw {
    amm_config: Pubkey,
    _pool_creator: Pubkey,
    token_0_vault: Pubkey,
    token_1_vault: Pubkey,
    _lp_mint: Pubkey,
    token_0_mint: Pubkey,
    token_1_mint: Pubkey,
    token_0_program: Pubkey,
    token_1_program: Pubkey,
    observation_key: Pubkey,
    _auth_bump: u8,
    _status: u8,
    _lp_mint_decimals: u8,
    mint_0_decimals: u8,
    mint_1_decimals: u8,
    protocol_fees_token_0: u64,
    protocol_fees_token_1: u64,
    fund_fees_token_0: u64,
    fund_fees_token_1: u64,
    creator_fee_on: u8,
    enable_creator_fee: bool,
    creator_fees_token_0: u64,
    creator_fees_token_1: u64,
}

fn decode_pool_state(account: &Account) -> Result<PoolStateRaw, RpcError> {
    let data = &account.data;
    if data.len() < 8 + 10 * 32 + 5 + 8 * 6 + 2 + 6 + 8 * 2 {
        return Err(RpcError::Decode("pool_state too short".into()));
    }
    let base = 8usize;
    let read_pk = |off: usize| -> Pubkey {
        let bytes: [u8; 32] = data[base + off..base + off + 32].try_into().unwrap();
        Pubkey::from(bytes)
    };
    let read_u64_abs = |abs: usize| -> u64 {
        u64::from_le_bytes(data[abs..abs + 8].try_into().unwrap())
    };
    let off = |i: usize| i * 32;
    let meta = base + off(10);
    let accrued_fees = meta + 5 + 8; // skip lp_supply
    let creator_meta = accrued_fees + 32 + 16; // skip protocol/fund fees + open_time + recent_epoch
    Ok(PoolStateRaw {
        amm_config: read_pk(off(0)),
        _pool_creator: read_pk(off(1)),
        token_0_vault: read_pk(off(2)),
        token_1_vault: read_pk(off(3)),
        _lp_mint: read_pk(off(4)),
        token_0_mint: read_pk(off(5)),
        token_1_mint: read_pk(off(6)),
        token_0_program: read_pk(off(7)),
        token_1_program: read_pk(off(8)),
        observation_key: read_pk(off(9)),
        _auth_bump: data[meta],
        _status: data[meta + 1],
        _lp_mint_decimals: data[meta + 2],
        mint_0_decimals: data[meta + 3],
        mint_1_decimals: data[meta + 4],
        protocol_fees_token_0: read_u64_abs(accrued_fees),
        protocol_fees_token_1: read_u64_abs(accrued_fees + 8),
        fund_fees_token_0: read_u64_abs(accrued_fees + 16),
        fund_fees_token_1: read_u64_abs(accrued_fees + 24),
        creator_fee_on: data[creator_meta],
        enable_creator_fee: data[creator_meta + 1] != 0,
        creator_fees_token_0: read_u64_abs(creator_meta + 8),
        creator_fees_token_1: read_u64_abs(creator_meta + 16),
    })
}

pub fn parse_commitment(s: &str) -> Result<CommitmentConfig, RpcError> {
    Ok(match s {
        "processed" => CommitmentConfig::processed(),
        "confirmed" => CommitmentConfig::confirmed(),
        "finalized" => CommitmentConfig::finalized(),
        other => return Err(RpcError::Decode(format!("unknown commitment: {other}"))),
    })
}

pub fn effective_vault_amounts(pool: &CpmmPool) -> (u64, u64) {
    let fees_0 = pool
        .protocol_fees_token_0
        .saturating_add(pool.fund_fees_token_0)
        .saturating_add(pool.creator_fees_token_0);
    let fees_1 = pool
        .protocol_fees_token_1
        .saturating_add(pool.fund_fees_token_1)
        .saturating_add(pool.creator_fees_token_1);
    (
        pool.vault_0_amount.saturating_sub(fees_0),
        pool.vault_1_amount.saturating_sub(fees_1),
    )
}
