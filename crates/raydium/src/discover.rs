//! On-chain CPMM pool discovery when Raydium API is slow or unreachable.

use crate::constants::CPMM_PROGRAM_ID;
use crate::pool::{MintInfo, PoolRef};
use crate::rpc::RpcPoolLoader;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_filter::{Memcmp, MemcmpEncodedBytes, RpcFilterType};
use solana_sdk::pubkey::Pubkey;
use tracing::{debug, info, warn};

/// Raydium CPMM `PoolState` account size on mainnet.
const CPMM_POOL_ACCOUNT_SIZE: u64 = 637;
const POOL_TOKEN_0_MINT_OFFSET: usize = 8 + 160;
const POOL_TOKEN_1_MINT_OFFSET: usize = 8 + 192;

impl RpcPoolLoader {
    /// All CPMM pools for `mint_a` / `mint_b` via RPC `getProgramAccounts`.
    pub async fn find_all_cpmm_pools(
        &self,
        mint_a: &Pubkey,
        mint_b: &Pubkey,
    ) -> Result<Vec<PoolRef>, crate::rpc::RpcError> {
        let (token0, token1) = if mint_a.to_bytes() < mint_b.to_bytes() {
            (mint_a, mint_b)
        } else {
            (mint_b, mint_a)
        };

        let accounts = self.query_pools_by_mints(token0, token1).await?;
        let count = accounts.len();
        if count == 0 {
            debug!(mint_a = %mint_a, mint_b = %mint_b, "rpc find_all_cpmm_pools: no accounts");
            return Ok(Vec::new());
        }

        info!(
            matches = count,
            mint_a = %mint_a,
            mint_b = %mint_b,
            "rpc find_all_cpmm_pools: found"
        );
        Ok(accounts
            .into_iter()
            .map(|pool_id| pool_ref_for_pair(pool_id, *mint_a, *mint_b))
            .collect())
    }

    async fn query_pools_by_mints(
        &self,
        token0: &Pubkey,
        token1: &Pubkey,
    ) -> Result<Vec<Pubkey>, crate::rpc::RpcError> {
        let config = RpcProgramAccountsConfig {
            filters: Some(vec![
                RpcFilterType::DataSize(CPMM_POOL_ACCOUNT_SIZE),
                RpcFilterType::Memcmp(Memcmp::new(
                    POOL_TOKEN_0_MINT_OFFSET,
                    MemcmpEncodedBytes::Base58(token0.to_string()),
                )),
                RpcFilterType::Memcmp(Memcmp::new(
                    POOL_TOKEN_1_MINT_OFFSET,
                    MemcmpEncodedBytes::Base58(token1.to_string()),
                )),
            ]),
            account_config: RpcAccountInfoConfig {
                data_slice: Some(solana_account_decoder_client_types::UiDataSliceConfig {
                    offset: 0,
                    length: 0,
                }),
                ..RpcAccountInfoConfig::default()
            },
            with_context: None,
            sort_results: None,
        };

        self.client()
            .get_program_ui_accounts_with_config(&CPMM_PROGRAM_ID, config)
            .await
            .map_err(|e| {
                warn!(
                    error = %e,
                    token0 = %token0,
                    token1 = %token1,
                    "rpc getProgramAccounts failed"
                );
                crate::rpc::RpcError::Client(e)
            })
            .map(|accounts| accounts.into_iter().map(|(pk, _)| pk).collect())
    }
}

fn pool_ref_for_pair(pool_id: Pubkey, mint_a: Pubkey, mint_b: Pubkey) -> PoolRef {
    PoolRef {
        pool_id,
        amm_config: None,
        mint_a: mint_info_guess(mint_a),
        mint_b: mint_info_guess(mint_b),
        trade_fee_rate: None,
        tvl: None,
    }
}

fn mint_info_guess(mint: Pubkey) -> MintInfo {
    use crate::constants::{NATIVE_MINT, TOKEN_PROGRAM_ID};
    let decimals = if mint == NATIVE_MINT { 9 } else { 6 };
    MintInfo {
        address: mint,
        token_program: TOKEN_PROGRAM_ID,
        decimals,
        symbol: String::new(),
    }
}
